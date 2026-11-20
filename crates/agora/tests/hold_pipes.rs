//! ADR-0093 (Unix): the holder keeps structured agents on pipes. A real `agora hold` and `/bin/sh` one-liners (the holder never reads
//! what a pipe agent writes beyond cutting it into lines, so any program that writes lines shows it); never a real agent.
//!
//! The "node" is this test process: the lock, `run/node.lock`, names it.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use agora_node::holder_link::{
    read_frame, socket_path, write_frame, AgentState, Client, LinkError, PipeData, Refusal, Reply,
    Request, StartPipeRequest, LINK_VERSION, LINK_VERSION_PIPES, MAX_FRAME,
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
        let rig = Rig { home, holder };
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
                Err(e) => panic!("connect: {e}"),
            }
        }
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
                Err(e) => panic!("connect: {e}"),
            }
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = self.holder.kill();
        let _ = self.holder.wait();
        let _ = std::fs::remove_dir_all(&self.home);
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
    let pid = c
        .start_pipe(&pipe(
            "p1",
            "echo ready; while read l; do echo \"got:$l\"; done",
        ))
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
        c.start_pipe(&pipe("p1", "sleep 1")),
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
    c.start_pipe(&pipe(
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
    a.start_pipe(&pipe(
        "p1",
        "echo first; read x; echo second; read y; echo third; sleep 600",
    ))
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
        c.start_pipe(&pipe("p1", "sleep 5")),
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
            let mut s = agora_platform::local_connect(&socket_path(&rig.home)).expect("connect");
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
            let (payload, _) = read_frame(&mut s).expect("frame").expect("an answer");
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
            match read_frame(s) {
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
    c.start_pipe(&pipe("p1", "while read l; do echo \"got:$l\"; done"))
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
