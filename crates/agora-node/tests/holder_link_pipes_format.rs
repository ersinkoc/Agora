//! Golden fixtures of the holder link's pipe agents (ADR-0093, `docs/specs/holder-link.md`, "Pipe agents"). As for the first fixtures
//! (`holder_link_format.rs`): a fixture is the hex of one frame's payload, a published one is never edited, only added. Set
//! `AGORA_NEW_FIXTURES=1` to write the files that are missing (it never overwrites one).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use agora_node::holder_link::{
    AgentInfo, AgentState, LinkError, PipeData, Reply, Request, StartPipeRequest,
    LINK_VERSION_PIPES,
};

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/holder-link")
        .join(format!("{name}.hex"))
}

fn hex(bytes: &[u8]) -> String {
    let mut s: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    s.push('\n');
    s
}

fn unhex(text: &str) -> Vec<u8> {
    let t: Vec<char> = text.trim().chars().collect();
    t.chunks(2)
        .map(|c| u8::from_str_radix(&c.iter().collect::<String>(), 16).expect("hex"))
        .collect()
}

fn stored(name: &str, now: &[u8]) -> Vec<u8> {
    let path = fixture(name);
    if !path.exists() && std::env::var_os("AGORA_NEW_FIXTURES").is_some() {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        std::fs::write(&path, hex(now)).expect("write fixture");
    }
    unhex(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("fixture {name}: {e}")))
}

fn start() -> StartPipeRequest {
    StartPipeRequest {
        agent: "a1".into(),
        argv: vec!["agora".into(), "acp".into()],
        cwd: Some("/work".into()),
        env: vec![("PATH".into(), "/usr/bin".into())],
    }
}

fn data() -> PipeData {
    PipeData {
        info: AgentInfo {
            agent: "a1".into(),
            pid: 4242,
            state: AgentState::Running,
        },
        dropped: 70_000,
        eof: false,
        seen: vec![br#"{"jsonrpc":"2.0","method":"approval/request"}"#.to_vec()],
        fresh: vec![b"one".to_vec(), b"two".to_vec()],
    }
}

#[test]
fn the_pipe_requests_match_their_fixtures_both_ways() {
    let cases: Vec<(&str, Request)> = vec![
        ("hello-v2", Request::Hello { version: 2 }),
        ("start-pipe", Request::StartPipe(start())),
        ("attach-pipes", Request::AttachPipes),
        ("pipe-read", Request::PipeRead { agent: "a1".into() }),
        (
            "pipe-write",
            Request::PipeWrite {
                agent: "a1".into(),
                line: br#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#.to_vec(),
            },
        ),
    ];
    for (name, request) in cases {
        let bytes = stored(name, &request.encode());
        assert_eq!(request.encode(), bytes, "{name}: encoding changed");
        assert_eq!(Request::decode(&bytes).expect(name), request, "{name}");
    }
}

#[test]
fn the_pipe_replies_match_their_fixtures_both_ways() {
    let cases: Vec<(&str, Reply)> = vec![
        (
            "hello-ok-v2",
            Reply::HelloOk {
                version: LINK_VERSION_PIPES,
                holder_pid: 99,
                holder_start: 123_456_789,
            },
        ),
        (
            "pipe-started",
            Reply::PipeStarted {
                agent: "a1".into(),
                pid: 4242,
            },
        ),
        ("pipe-data", Reply::PipeData(data())),
        (
            "pipe-data-exited",
            Reply::PipeData(PipeData {
                info: AgentInfo {
                    agent: "a1".into(),
                    pid: 4242,
                    state: AgentState::Exited {
                        code: Some(0),
                        signal: None,
                    },
                },
                dropped: 0,
                eof: true,
                seen: Vec::new(),
                fresh: Vec::new(),
            }),
        ),
    ];
    for (name, reply) in cases {
        let bytes = stored(name, &reply.encode());
        assert_eq!(reply.encode(), bytes, "{name}: encoding changed");
        assert_eq!(Reply::decode(&bytes).expect(name), reply, "{name}");
    }
}

/// A frame cut anywhere, with something left over, with a count larger than the frame or with a line that claims more than is there is
/// refused, never guessed; so is a flag that is neither 0 nor 1.
#[test]
fn damaged_pipe_frames_are_refused_not_guessed() {
    let good = Reply::PipeData(data()).encode();
    for cut in 0..good.len() {
        assert!(Reply::decode(&good[..cut]).is_err(), "cut at {cut}");
    }
    let mut extra = good.clone();
    extra.push(0);
    assert!(matches!(
        Reply::decode(&extra),
        Err(LinkError::Malformed(_))
    ));
    let write = Request::PipeWrite {
        agent: "a1".into(),
        line: b"xy".to_vec(),
    }
    .encode();
    for cut in 0..write.len() {
        assert!(Request::decode(&write[..cut]).is_err(), "cut at {cut}");
    }
    // A line count larger than the bytes that are left, and a line that claims more bytes than are left.
    let mut lie = vec![72, 0, 0, 0, 1, b'a', 0, 0, 0, 0, 0];
    lie.extend_from_slice(&[0; 8]); // dropped
    lie.push(0); // eof
    lie.extend_from_slice(&[0xff, 0xff, 0xff, 0xff]); // seen: a count nothing can satisfy
    assert!(Reply::decode(&lie).is_err());
    let mut long_line = vec![12, 0, 0, 0, 1, b'a'];
    long_line.extend_from_slice(&[0x7f, 0xff, 0xff, 0xff, b'x']);
    assert!(Request::decode(&long_line).is_err());
    // A flag that is neither 0 nor 1.
    let mut flag = Reply::PipeData(PipeData {
        info: AgentInfo {
            agent: "a".into(),
            pid: 1,
            state: AgentState::Running,
        },
        dropped: 0,
        eof: false,
        seen: Vec::new(),
        fresh: Vec::new(),
    })
    .encode();
    // The flag is the byte before the two (empty) line counts.
    let at = flag.len() - 4 - 4 - 1;
    flag[at] = 7;
    assert!(Reply::decode(&flag).is_err());
}
