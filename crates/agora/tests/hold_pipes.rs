//! ADR-0093 (Unix): the holder keeps structured agents on pipes. A real `agora hold` and `/bin/sh` one-liners (the holder never reads
//! what a pipe agent writes beyond cutting it into lines, so any program that writes lines shows it); never a real agent.
//!
//! The "node" is this test process: the lock, `run/node.lock`, names it.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use agora_node::holder_link::{
    read_frame, socket_path, write_frame, AgentState, Client, LinkError, PipeData, Refusal, Reply,
    Request, StartPipeRequest, LINK_VERSION, LINK_VERSION_PIPES, MAX_AGENT_ID, MAX_FRAME,
};

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

fn temp_home(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "agora-holdp-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("home");
    dir
}

struct Rig {
    home: PathBuf,
    holder: Child,
    /// The agents this rig's holder started for its test, as identified when they started (#1014): ended in `Drop` after the holder,
    /// so a failed test leaves none running.
    agents: Mutex<Vec<agora_platform::ProcessIdentity>>,
}

/// The holder makes its socket file a moment before it listens on it: a connection in between is refused (macOS answers
/// `ECONNREFUSED`, #830). The node's own path retries it the same way (`HolderHub::open`); the rig waits for the file only.
fn not_listening_yet(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::ConnectionRefused
}

impl Rig {
    fn new(tag: &str) -> Rig {
        let home = temp_home(tag);
        agora_platform::ensure_private_dir(&home.join("run")).expect("run folder");
        let me = std::process::id();
        let start = agora_platform::process_start_time(me).expect("own start time");
        std::fs::write(home.join("run/node.lock"), format!("{me} {start} test\n")).expect("lock");
        let holder = Command::new(AGORA)
            .arg("hold")
            .arg("--home")
            .arg(&home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start agora hold");
        let rig = Rig {
            home,
            holder,
            agents: Mutex::new(Vec::new()),
        };
        let end = Instant::now() + Duration::from_secs(15);
        while !socket_path(&rig.home).exists() {
            assert!(Instant::now() < end, "the holder never listened");
            std::thread::sleep(Duration::from_millis(20));
        }
        rig
    }

    /// Connects, waiting out the moment before the holder has noticed that the node before this one is gone.
    fn connect(&self) -> Client {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            match Client::connect(&socket_path(&self.home)) {
                Ok(c) => return c,
                Err(LinkError::Refused(Refusal::Busy, _)) if Instant::now() < end => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(LinkError::Io(e)) if not_listening_yet(&e) && Instant::now() < end => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => panic!("connect: {e}"),
            }
        }
    }

    /// Starts a pipe agent through `c` and records it to be ended with the rig (#1014). Every test starts its agents here.
    fn start(&self, c: &mut Client, request: &StartPipeRequest) -> Result<u32, LinkError> {
        let pid = c.start_pipe(request)?;
        if let Some(id) = self.ours(pid, request) {
            self.agents
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(id);
        }
        Ok(pid)
    }

    /// The identity of the agent the holder just started as `pid`, read first and then checked as #1030 does: it must name the
    /// program asked for, and that pid must still be the holder's child. `None` (nothing is recorded, so nothing is ever signalled by
    /// its bare number) when it cannot be identified, has ended, or is not it.
    fn ours(
        &self,
        pid: u32,
        request: &StartPipeRequest,
    ) -> Option<agora_platform::ProcessIdentity> {
        let id = agora_platform::process_identity(pid).ok()?;
        let argv0 = request.argv.first()?;
        let program = std::path::Path::new(argv0).file_name()?;
        // macOS's `/bin/sh` is a launcher that runs the shell chosen for it in its place, under that shell's name.
        let named = std::ffi::OsStr::new(&id.name) == program
            || (argv0 == "/bin/sh" && ["bash", "dash", "zsh"].contains(&id.name.as_str()));
        let is_child =
            agora_platform::process_children(self.holder.id()).is_ok_and(|c| c.contains(&id.pid));
        (named && is_child).then_some(id)
    }

    /// A node of this link version and nothing newer.
    fn connect_v1(&self) -> Client {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            match Client::connect_with_version(&socket_path(&self.home), LINK_VERSION) {
                Ok(c) => return c,
                Err(LinkError::Refused(Refusal::Busy, _)) if Instant::now() < end => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(LinkError::Io(e)) if not_listening_yet(&e) && Instant::now() < end => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => panic!("connect: {e}"),
            }
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = self.holder.kill();
        let _ = self.holder.wait();
        // The agents outlive their holder by design (ADR-0093); a test that failed before it stopped them left them running. Each
        // is ended only while it is still the process identified at its start: same pid, start time and boot (it may have replaced
        // its program since, as `sh -c` does with its last command), and with it its process group, which it leads (setsid).
        let agents = std::mem::take(self.agents.get_mut().unwrap_or_else(|e| e.into_inner()));
        for id in agents {
            if let Ok(now) = agora_platform::process_identity(id.pid) {
                if now.started == id.started && now.boot == id.boot {
                    let _ = agora_platform::kill_process_if_same(&now);
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// #1014: the rig records the agent it starts, and ending the rig ends it, though the agent outlives its holder by design.
#[test]
fn the_rig_ends_the_agents_it_started_when_it_ends() {
    if agora_platform::process_identity(std::process::id()).is_err() {
        return;
    }
    let rig = Rig::new("rig-ends");
    let mut c = rig.connect();
    c.attach().expect("attach");
    c.attach_pipes().expect("attach pipes");
    let pid = rig
        .start(&mut c, &pipe("p1", "echo up; sleep 600"))
        .expect("start");
    read_until(&mut c, "p1", "up");
    let recorded = rig.agents.lock().expect("agents").clone();
    assert_eq!(
        recorded.iter().map(|a| a.pid).collect::<Vec<_>>(),
        [pid],
        "the agent was not recorded; it reads as {:?}",
        agora_platform::process_identity(pid)
    );
    drop(c);
    drop(rig);
    let same = |now: &agora_platform::ProcessIdentity| {
        now.started == recorded[0].started && now.boot == recorded[0].boot
    };
    let end = Instant::now() + Duration::from_secs(5);
    while agora_platform::process_identity(pid).is_ok_and(|now| same(&now)) {
        assert!(Instant::now() < end, "the agent outlived the rig");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn pipe(agent: &str, script: &str) -> StartPipeRequest {
    StartPipeRequest {
        agent: agent.into(),
        argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
        cwd: None,
        env: vec![("PATH".into(), "/usr/bin:/bin".into())],
    }
}

fn text(lines: &[Vec<u8>]) -> Vec<String> {
    lines
        .iter()
        .map(|l| String::from_utf8_lossy(l).into_owned())
        .collect()
}

/// Reads until a fresh line equals `want`; returns every fresh line seen.
fn read_until(c: &mut Client, agent: &str, want: &str) -> Vec<String> {
    let end = Instant::now() + Duration::from_secs(20);
    let mut all = Vec::new();
    loop {
        let d = c.pipe_read(agent).expect("read");
        all.extend(text(&d.fresh));
        if all.iter().any(|l| l == want) {
            return all;
        }
        assert!(Instant::now() < end, "never saw {want:?}; got {all:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_pipe_agent_reads_the_lines_it_is_written_and_its_output_comes_back_as_lines() {
    let rig = Rig::new("lines");
    let mut c = rig.connect();
    assert_eq!(c.version, LINK_VERSION_PIPES);
    assert!(c.attach().expect("attach").is_empty());
    assert!(c.attach_pipes().expect("attach pipes").is_empty());
    let pid = rig
        .start(
            &mut c,
            &pipe("p1", "echo ready; while read l; do echo \"got:$l\"; done"),
        )
        .expect("start");
    assert!(pid > 1);
    read_until(&mut c, "p1", "ready");
    c.pipe_write("p1", b"{\"a\":1}").expect("write");
    c.pipe_write("p1", b"two").expect("write");
    let got = read_until(&mut c, "p1", "got:two");
    assert!(got.contains(&"got:{\"a\":1}".to_string()), "{got:?}");
    // It is in the list as running, with its own process.
    let list = c.list().expect("list");
    assert!(list
        .iter()
        .any(|a| a.agent == "p1" && a.pid == pid && a.state == AgentState::Running));
    // A running agent cannot be released; an unknown one is unknown.
    assert!(matches!(
        c.release("p1"),
        Err(LinkError::Refused(Refusal::BadRequest, _))
    ));
    assert!(matches!(
        c.pipe_write("nobody", b"x"),
        Err(LinkError::Refused(Refusal::UnknownAgent, _))
    ));
    // An id in use is refused.
    assert!(matches!(
        rig.start(&mut c, &pipe("p1", "sleep 1")),
        Err(LinkError::Refused(Refusal::AgentExists, _))
    ));
    // Stopping ends its process group; its end is reported and it is forgotten on release, and the holder ends with its last agent.
    c.stop("p1", true).expect("stop");
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let list = c.list().expect("list");
        if list
            .iter()
            .any(|a| a.agent == "p1" && matches!(a.state, AgentState::Exited { .. }))
        {
            break;
        }
        assert!(Instant::now() < end, "it did not end");
        std::thread::sleep(Duration::from_millis(20));
    }
    c.release("p1").expect("release");
    // It was the last: the holder ends, and the link with it.
    assert!(c.pipe_read("p1").is_err());
}

/// ADR-0093 decision 1: what a pipe agent wrote that no node read is kept up to 1 MiB, the oldest whole lines dropped first, and the
/// next node is told how many bytes went. Nothing is lost silently: kept bytes plus dropped bytes are all the agent wrote.
#[test]
fn output_nobody_reads_is_kept_to_a_mebibyte_and_the_dropped_bytes_are_counted() {
    let rig = Rig::new("overflow");
    let mut c = rig.connect();
    c.attach().expect("attach");
    c.attach_pipes().expect("attach pipes");
    // 3000 lines of 999 bytes: 2 997 000 bytes, nearly three times what is kept.
    rig.start(&mut c, &pipe(
        "p1",
        "i=0; while [ $i -lt 3000 ]; do printf '%0999d\\n' $i; i=$((i+1)); done; echo done-writing; sleep 600",
    ))
    .expect("start");
    // The node goes away and reads nothing; the agent writes all of it in the meantime.
    drop(c);
    std::thread::sleep(Duration::from_millis(1500));
    let end = Instant::now() + Duration::from_secs(30);
    let (mut kept, mut dropped, mut most) = (0u64, 0u64, 0usize);
    let last = loop {
        let mut c = rig.connect();
        c.attach().expect("attach");
        let held = c.attach_pipes().expect("attach pipes");
        assert_eq!(held.len(), 1);
        let p: &PipeData = &held[0];
        let bytes: usize = p.fresh.iter().map(Vec::len).sum();
        // What this hand-over holds is within the bound; every hand-over says what it dropped.
        assert!(bytes <= 1 << 20, "a hand-over of {bytes} bytes");
        most = most.max(bytes);
        kept += bytes as u64;
        dropped += p.dropped;
        if p.fresh
            .last()
            .is_some_and(|l| l.as_slice() == b"done-writing")
        {
            break p.fresh.clone();
        }
        drop(c);
        assert!(Instant::now() < end, "the agent never finished writing");
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(dropped > 0, "nothing was reported dropped (most {most})");
    // Whole lines only: every kept line is a full line of numbers, or the closing line.
    for l in &last[..last.len() - 1] {
        assert_eq!(l.len(), 999, "a partial line was kept");
    }
    // The newest are kept: the last numbered line is 2999.
    let n = &last[last.len() - 2];
    assert_eq!(String::from_utf8_lossy(n).trim_start_matches('0'), "2999");
    // Everything is accounted for: bytes taken and bytes dropped are every byte written (the closing line included).
    assert_eq!(kept + dropped, 3000 * 999 + "done-writing".len() as u64);
}

/// ADR-0093 decision 3: a node that comes after another is given the lines the first had (so what waited on a person can be asked
/// again) apart from the ones no node had, and what it takes is not given as new again.
#[test]
fn a_later_node_gets_what_the_first_had_as_seen_and_what_nobody_had_as_new() {
    let rig = Rig::new("replay");
    let mut a = rig.connect();
    a.attach().expect("attach");
    a.attach_pipes().expect("attach pipes");
    rig.start(
        &mut a,
        &pipe(
            "p1",
            "echo first; read x; echo second; read y; echo third; sleep 600",
        ),
    )
    .expect("start");
    read_until(&mut a, "p1", "first");
    // Node `a` goes away having read `first`; the agent writes `second` while no node is there.
    a.pipe_write("p1", b"go").expect("write");
    drop(a);
    std::thread::sleep(Duration::from_millis(300));
    let mut b = rig.connect();
    b.attach().expect("attach");
    let held = b.attach_pipes().expect("attach pipes");
    assert_eq!(held.len(), 1);
    assert_eq!(text(&held[0].seen), ["first"]);
    assert_eq!(text(&held[0].fresh), ["second"]);
    assert_eq!(held[0].dropped, 0);
    // The next one is given both as seen, and nothing as new; and a live read finds only what is written from now on.
    drop(b);
    std::thread::sleep(Duration::from_millis(300));
    let mut c = rig.connect();
    c.attach().expect("attach");
    let held = c.attach_pipes().expect("attach pipes");
    assert_eq!(text(&held[0].seen), ["first", "second"]);
    assert!(held[0].fresh.is_empty());
    c.pipe_write("p1", b"go").expect("write");
    read_until(&mut c, "p1", "third");
}

/// A node of link version 1 (built before pipe agents) is never sent the new tags and is refused if it asks for them, and what it did
/// before works as it did.
#[test]
fn a_node_of_link_version_one_is_refused_pipes_and_keeps_everything_else() {
    let rig = Rig::new("v1");
    let mut c = rig.connect_v1();
    assert_eq!(c.version, LINK_VERSION);
    c.attach().expect("attach");
    assert!(matches!(
        rig.start(&mut c, &pipe("p1", "sleep 5")),
        Err(LinkError::Refused(Refusal::BadRequest, _))
    ));
    assert!(matches!(
        c.attach_pipes(),
        Err(LinkError::Refused(Refusal::BadRequest, _))
    ));
    assert!(c.list().expect("list").is_empty());
    // A version nobody speaks is still refused, with the holder's own version.
    drop(c);
    std::thread::sleep(Duration::from_millis(300));
    match Client::connect_with_version(&socket_path(&rig.home), LINK_VERSION_PIPES + 1) {
        Err(LinkError::Refused(Refusal::VersionMismatch, v)) => assert_eq!(v, LINK_VERSION),
        other => panic!("expected a version refusal, got {:?}", other.map(|_| ())),
    }
}

/// The link refuses what is too large or not a message, closes, and keeps serving the next node; a line too large for one frame
/// fails that write only, never the link.
#[test]
fn oversize_and_garbage_frames_are_refused_and_the_holder_keeps_serving() {
    let rig = Rig::new("garbage");
    let raw = |rig: &Rig| {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let mut s = match agora_platform::local_connect(&socket_path(&rig.home)) {
                Ok(s) => s,
                Err(e) if not_listening_yet(&e) && Instant::now() < end => {
                    std::thread::sleep(Duration::from_millis(25));
                    continue;
                }
                Err(e) => panic!("connect: {e:?}"),
            };
            write_frame(
                &mut s,
                &Request::Hello {
                    version: LINK_VERSION_PIPES,
                }
                .encode(),
                None,
            )
            .expect("hello");
            s.set_read_timeout(Some(Duration::from_secs(5)))
                .expect("timeout");
            let (payload, _) = read_frame(&mut s, Duration::from_secs(5))
                .expect("frame")
                .expect("an answer");
            match Reply::decode(&payload).expect("reply") {
                Reply::HelloOk { .. } => return s,
                Reply::Refused {
                    code: Refusal::Busy,
                    ..
                } if Instant::now() < end => std::thread::sleep(Duration::from_millis(50)),
                other => panic!("{other:?}"),
            }
        }
    };
    let closed = |s: &mut agora_platform::LocalStream| {
        // The holder closes a link it will not serve: what is read next is the end of it, or an error, never an answer.
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            match read_frame(s, Duration::from_millis(200)) {
                Err(_) => return true,
                Ok(Some(_)) => return false,
                Ok(None) if Instant::now() < end => {}
                Ok(None) => return false,
            }
        }
    };
    // 1. A length larger than a frame may be: refused at once, before any of the claimed bytes arrive.
    let mut s = raw(&rig);
    let too_big = u32::try_from(MAX_FRAME + 1).unwrap().to_be_bytes();
    std::io::Write::write_all(&mut s, &too_big).expect("write length");
    assert!(closed(&mut s), "an oversize frame was not refused");
    drop(s);
    std::thread::sleep(Duration::from_millis(300));
    // 2. A length of zero.
    let mut s = raw(&rig);
    std::io::Write::write_all(&mut s, &0u32.to_be_bytes()).expect("write length");
    assert!(closed(&mut s), "an empty frame was not refused");
    drop(s);
    std::thread::sleep(Duration::from_millis(300));
    // 3. A tag nobody knows, and 4. a known tag with a cut body.
    for payload in [vec![200u8, 1, 2, 3], vec![12u8, 0, 0, 0, 5, b'a']] {
        let mut s = raw(&rig);
        write_frame(&mut s, &payload, None).expect("garbage");
        assert!(closed(&mut s), "garbage was answered: {payload:?}");
        drop(s);
        std::thread::sleep(Duration::from_millis(300));
    }
    // The holder serves the next node as if nothing happened.
    let mut c = rig.connect();
    c.attach().expect("attach");
    c.attach_pipes().expect("attach pipes");
    rig.start(
        &mut c,
        &pipe("p1", "while read l; do echo \"got:$l\"; done"),
    )
    .expect("start");
    // A line that cannot be one frame is refused by the sender before anything is sent: the link is whole after.
    let big = vec![b'x'; MAX_FRAME];
    assert!(matches!(
        c.pipe_write("p1", &big),
        Err(LinkError::Malformed(_))
    ));
    c.pipe_write("p1", b"after").expect("write after");
    read_until(&mut c, "p1", "got:after");
}

/// A flood of one-byte lines fills the buffer with more than a million lines: the hand-over of what is kept must still fit a frame, so
/// the link stays whole (it used to be closed for every agent, on every reconnect), and another agent keeps working.
#[test]
fn a_flood_of_tiny_lines_cannot_make_a_reply_larger_than_a_frame() {
    let rig = Rig::new("tiny");
    let mut c = rig.connect();
    c.attach().expect("attach");
    c.attach_pipes().expect("attach pipes");
    rig.start(
        &mut c,
        &pipe("calm", "while read l; do echo \"got:$l\"; done"),
    )
    .expect("start calm");
    // `yes` writes "y" lines as fast as the pipe takes them; 1.5 million of them.
    rig.start(
        &mut c,
        &pipe(
            "flood",
            "yes | head -n 1500000; echo done-writing; sleep 600",
        ),
    )
    .expect("start flood");
    drop(c);
    std::thread::sleep(Duration::from_millis(1500));
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let mut c = rig.connect();
        c.attach().expect("attach");
        let held = c.attach_pipes().expect("the hand-over must be sendable");
        assert_eq!(held.len(), 2);
        let flood = held.iter().find(|p| p.info.agent == "flood").unwrap();
        let encoded: usize = flood
            .seen
            .iter()
            .chain(&flood.fresh)
            .map(|l| l.len() + agora_platform::LINE_COST)
            .sum();
        assert!(encoded <= agora_platform::HELD_BUFFER, "{encoded}");
        let finished = flood
            .fresh
            .last()
            .is_some_and(|l| l.as_slice() == b"done-writing");
        // The other agent still works, on this link.
        c.pipe_write("calm", b"hello").expect("write");
        read_until(&mut c, "calm", "got:hello");
        if finished {
            break;
        }
        drop(c);
        assert!(Instant::now() < end, "the flood never finished");
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// A process the agent started and left behind does not outlive it: when the holder collects the agent's end it ends what is left of
/// the agent's group, so the output closes and nothing runs on unseen.
#[test]
fn what_an_ended_agent_left_in_its_group_is_ended_with_it() {
    let rig = Rig::new("orphan");
    let mut c = rig.connect();
    c.attach().expect("attach");
    c.attach_pipes().expect("attach pipes");
    // The child holds the agent's output open and would run for ten minutes; the agent says its pid and ends.
    rig.start(&mut c, &pipe("p1", "sleep 600 & echo $!; exit 0"))
        .expect("start");
    let lines = read_until_any(&mut c, "p1");
    let child: u32 = lines[0].parse().expect("the child's pid");
    let end = Instant::now() + Duration::from_secs(10);
    // A killed process that nobody has collected yet may still show; what counts is that it is no longer running.
    let running = |pid: u32| {
        let out = Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .expect("ps");
        let stat = String::from_utf8_lossy(&out.stdout).trim().to_string();
        !stat.is_empty() && !stat.starts_with('Z')
    };
    while running(child) {
        assert!(Instant::now() < end, "the agent's child still runs");
        std::thread::sleep(Duration::from_millis(50));
    }
    // And the holder reports the agent ended, with its output closed.
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let d = c.pipe_read("p1").expect("read");
        if matches!(d.info.state, AgentState::Exited { .. }) && d.eof {
            break;
        }
        assert!(Instant::now() < end, "the output never closed");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// F1301: what the agent moved into a process group of its own (job control, `setpgid`) is still in the session the holder made for
/// it, and is ended with it too, so it neither runs on unnamed nor holds the agent's output open. Where the holder cannot see sessions
/// (not Linux) only the group is ended, and this is not checked.
#[test]
fn what_an_ended_agent_moved_to_a_group_of_its_own_is_ended_with_it() {
    if std::env::consts::OS != "linux" {
        return;
    }
    let rig = Rig::new("session");
    let mut c = rig.connect();
    c.attach().expect("attach");
    c.attach_pipes().expect("attach pipes");
    // The child moves to a group of its own and only then lets the agent go on (a pipe it closes), so the agent's end is collected
    // after the move; it holds the agent's output open and would run for ten minutes.
    let perl = "pipe(R,W); my $p=fork(); if($p==0){ close R; setpgrp(0,0); close W; exec('sleep','600'); } close W; my $x=<R>; print \"$p\\n\";";
    rig.start(
        &mut c,
        &StartPipeRequest {
            agent: "p1".into(),
            argv: vec!["/bin/sh".into(), "-c".into(), "perl -e \"$P\"".into()],
            cwd: None,
            env: vec![
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("P".into(), perl.into()),
            ],
        },
    )
    .expect("start");
    let lines = read_until_any(&mut c, "p1");
    let child: u32 = lines[0].parse().expect("the child's pid");
    let running = |pid: u32| {
        let out = Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .expect("ps");
        let stat = String::from_utf8_lossy(&out.stdout).trim().to_string();
        !stat.is_empty() && !stat.starts_with('Z')
    };
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let d = c.pipe_read("p1").expect("read");
        if matches!(d.info.state, AgentState::Exited { .. }) && d.eof && !running(child) {
            break;
        }
        if Instant::now() >= end {
            let survived = running(child);
            if survived {
                // The test's own leftover, never anything else.
                let _ = Command::new("kill")
                    .args(["-KILL", &child.to_string()])
                    .status();
            }
            panic!("the agent's moved child still runs ({survived}) or its output never closed");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// F1300: once the holder collected an agent's end its input is closed: a line written to it is `Failed` (the first one too), never
/// queued for nobody and lost; a released one is unknown, and a running one still gets its lines.
#[test]
fn a_line_for_an_agent_that_ended_is_refused_never_queued_and_lost() {
    let rig = Rig::new("ended-input");
    let mut c = rig.connect();
    c.attach().expect("attach");
    c.attach_pipes().expect("attach pipes");
    rig.start(
        &mut c,
        &pipe("live", "while read l; do echo \"got:$l\"; done"),
    )
    .expect("start live");
    rig.start(&mut c, &pipe("gone", "echo bye"))
        .expect("start gone");
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let d = c.pipe_read("gone").expect("read");
        if d.eof && matches!(d.info.state, AgentState::Exited { .. }) {
            break;
        }
        assert!(Instant::now() < end, "it never ended");
        std::thread::sleep(Duration::from_millis(20));
    }
    for i in 0..3 {
        assert!(
            matches!(
                c.pipe_write("gone", format!("l{i}").as_bytes()),
                Err(LinkError::Refused(Refusal::Failed, _))
            ),
            "write {i} to an ended agent was not refused"
        );
    }
    c.release("gone").expect("release");
    assert!(matches!(
        c.pipe_write("gone", b"x"),
        Err(LinkError::Refused(Refusal::UnknownAgent, _))
    ));
    c.pipe_write("live", b"still")
        .expect("a running agent's write");
    read_until(&mut c, "live", "got:still");
}

fn read_until_any(c: &mut Client, agent: &str) -> Vec<String> {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let d = c.pipe_read(agent).expect("read");
        let lines = text(&d.fresh);
        if !lines.is_empty() {
            return lines;
        }
        assert!(Instant::now() < end, "nothing was written");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A node that speaks the newest version meets a holder built before pipe agents (it answers `Hello(2)` with `VersionMismatch` and its own
/// version 1): it says `Hello(1)` again on a new connection and works at version 1.
#[test]
fn a_new_node_falls_back_to_version_one_against_a_holder_that_only_speaks_it() {
    let home = temp_home("v1holder");
    let socket = home.join("old.sock");
    let listener = agora_platform::local_listen(&socket).expect("listen");
    // The node checks who serves the endpoint (M2.6, T79): this test's own process, which `HelloOk` must name, and still connected when
    // it asks (macOS answers ENOTCONN for a peer that already closed), so the last connection stays open until the node is done.
    let (done, client_done) = std::sync::mpsc::channel::<()>();
    let server = std::thread::spawn(move || {
        let mut seen = Vec::new();
        let mut last = None;
        for _ in 0..2 {
            let mut s = listener.accept().expect("accept");
            s.set_read_timeout(Some(Duration::from_secs(5))).expect("t");
            let (payload, _) = read_frame(&mut s, Duration::from_secs(5))
                .expect("frame")
                .expect("hello");
            let Request::Hello { version } = Request::decode(&payload).expect("hello") else {
                panic!("not a hello");
            };
            seen.push(version);
            let reply = if version == LINK_VERSION {
                Reply::HelloOk {
                    version: LINK_VERSION,
                    holder_pid: std::process::id(),
                    holder_start: 2,
                }
            } else {
                Reply::Refused {
                    code: Refusal::VersionMismatch,
                    holder_version: LINK_VERSION,
                }
            };
            write_frame(&mut s, &reply.encode(), None).expect("reply");
            last = Some(s);
        }
        let _ = client_done.recv_timeout(Duration::from_secs(10));
        drop(last);
        seen
    });
    let c = Client::connect(&socket).expect("falls back");
    let _ = done.send(());
    assert_eq!(c.version, LINK_VERSION);
    assert_eq!(
        server.join().expect("server"),
        [LINK_VERSION_PIPES, LINK_VERSION]
    );
    let _ = std::fs::remove_dir_all(home);
}

/// #929: a pipe agent's id is bounded as a terminal agent's: a longer one is refused on `StartPipe` and nothing starts; one at the
/// bound starts, is listed, and is handed over by `AttachPipes`.
#[test]
fn a_pipe_agent_id_longer_than_the_bound_is_refused_and_one_at_the_bound_is_handed_over() {
    let rig = Rig::new("idlen");
    let mut c = rig.connect();
    assert!(c.attach().expect("attach").is_empty());
    for long in ["p".repeat(MAX_AGENT_ID + 1), "p".repeat(3 << 20)] {
        match rig.start(&mut c, &pipe(&long, "echo up; sleep 600")) {
            Err(LinkError::Refused(Refusal::BadRequest, _)) => {}
            Err(e) => panic!("refused for another reason: {e}"),
            Ok(pid) => {
                let _ = c.stop(&long, true);
                panic!("an id of {} bytes was started (pid {pid})", long.len());
            }
        }
    }
    assert!(c.list().expect("list").is_empty(), "nothing started");

    let at = "q".repeat(MAX_AGENT_ID);
    let pid = rig
        .start(&mut c, &pipe(&at, "echo up; sleep 600"))
        .expect("start");
    read_until(&mut c, &at, "up");
    let listed = c.list().expect("list");
    assert_eq!(
        (listed[0].agent.as_str(), listed[0].pid),
        (at.as_str(), pid)
    );
    drop(c);
    let mut c = rig.connect();
    assert!(c.attach().expect("attach").is_empty());
    let held = c.attach_pipes().expect("attach pipes");
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].info.agent, at);
    c.stop(&at, true).expect("stop");
}
