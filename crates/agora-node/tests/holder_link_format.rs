//! Golden fixtures of the holder link (ADR-0079 decision 5, `docs/specs/holder-link.md`). A fixture is the hex of one frame's payload.
//! A published fixture is never edited, only added: when a message changes meaning, a new message gets a new tag and a new fixture.
//! Set `AGORA_NEW_FIXTURES=1` to write the files that are missing (it never overwrites one).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use agora_node::holder_link::{
    agent_id_fits, AgentInfo, AgentState, LinkError, PipeData, Refusal, Reply, Request,
    StartRequest, TerminalInfo, LINK_VERSION, MAX_AGENT_ID, MAX_FRAME, MAX_LIST_ENTRY,
};
use agora_platform::HeldSignal;

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

/// The stored bytes of fixture `name`; written first when missing and asked for.
fn stored(name: &str, now: &[u8]) -> Vec<u8> {
    let path = fixture(name);
    if !path.exists() && std::env::var_os("AGORA_NEW_FIXTURES").is_some() {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        std::fs::write(&path, hex(now)).expect("write fixture");
    }
    unhex(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("fixture {name}: {e}")))
}

fn start() -> StartRequest {
    StartRequest {
        agent: "a1".into(),
        argv: vec!["claude".into(), "--resume".into()],
        cwd: Some("/work".into()),
        env: vec![("TERM".into(), "xterm".into())],
        env_remove: vec!["CLAUDE_CODE_*".into()],
        cols: 120,
        rows: 40,
        cpu_seconds: Some(60),
        memory_bytes: None,
    }
}

fn info(state: AgentState) -> AgentInfo {
    AgentInfo {
        agent: "a1".into(),
        pid: 4242,
        state,
    }
}

#[test]
fn every_request_matches_its_fixture_both_ways() {
    let cases: Vec<(&str, Request)> = vec![
        ("hello-v1", Request::Hello { version: 1 }),
        ("attach", Request::Attach),
        ("start", Request::Start(start())),
        (
            "resize",
            Request::Resize {
                agent: "a1".into(),
                cols: 100,
                rows: 30,
            },
        ),
        (
            "signal-hangup",
            Request::Signal {
                agent: "a1".into(),
                signal: HeldSignal::Hangup,
            },
        ),
        (
            "stop-force",
            Request::Stop {
                agent: "a1".into(),
                force: true,
            },
        ),
        ("list", Request::List),
        ("release", Request::Release { agent: "a1".into() }),
    ];
    for (name, request) in cases {
        let bytes = stored(name, &request.encode());
        assert_eq!(request.encode(), bytes, "{name}: encoding changed");
        assert_eq!(Request::decode(&bytes).expect(name), request, "{name}");
    }
}

#[test]
fn every_reply_matches_its_fixture_both_ways() {
    let cases: Vec<(&str, Reply)> = vec![
        (
            "hello-ok",
            Reply::HelloOk {
                version: 1,
                holder_pid: 99,
                holder_start: 123_456_789,
            },
        ),
        (
            "refused-version",
            Reply::Refused {
                code: Refusal::VersionMismatch,
                holder_version: 1,
            },
        ),
        (
            "refused-busy",
            Reply::Refused {
                code: Refusal::Busy,
                holder_version: 1,
            },
        ),
        ("ok", Reply::Ok),
        (
            "started",
            Reply::Started {
                agent: "a1".into(),
                pid: 4242,
            },
        ),
        (
            "agents",
            Reply::Agents(vec![
                info(AgentState::Running),
                info(AgentState::Exited {
                    code: Some(0),
                    signal: None,
                }),
            ]),
        ),
        (
            "terminal-exited",
            Reply::Terminal(TerminalInfo {
                info: info(AgentState::Exited {
                    code: None,
                    signal: Some(9),
                }),
                buffer_dropped: true,
                buffer: b"bye\r\n".to_vec(),
            }),
        ),
        ("attach-done", Reply::AttachDone { count: 3 }),
    ];
    for (name, reply) in cases {
        let bytes = stored(name, &reply.encode());
        assert_eq!(reply.encode(), bytes, "{name}: encoding changed");
        assert_eq!(Reply::decode(&bytes).expect(name), reply, "{name}");
    }
}

#[test]
fn the_hello_and_the_refusal_are_frozen() {
    // A node and a holder of any two versions must be able to say that they differ: these two frames never change.
    assert_eq!(Request::Hello { version: 7 }.encode(), [1, 0, 7]);
    assert_eq!(
        Reply::Refused {
            code: Refusal::VersionMismatch,
            holder_version: LINK_VERSION
        }
        .encode(),
        [65, 1, 0, 1]
    );
}

#[test]
fn damaged_frames_are_refused_not_guessed() {
    let good = Request::Start(start()).encode();
    for cut in 0..good.len() {
        assert!(Request::decode(&good[..cut]).is_err(), "cut at {cut}");
    }
    let mut extra = good.clone();
    extra.push(0);
    assert!(matches!(
        Request::decode(&extra),
        Err(LinkError::Malformed(_))
    ));
    // An unknown tag, an unknown signal and a count larger than the frame.
    assert!(Request::decode(&[200]).is_err());
    assert!(Request::decode(&[5, 0, 0, 0, 1, b'a', 9]).is_err());
    assert!(Request::decode(&[3, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]).is_err());
    assert!(Reply::decode(&[65, 99, 0, 1]).is_err());
}

/// #929: every id the node makes fits the bound, and with an id at the bound the largest replies that name an agent fit a frame: a
/// `Terminal` with a whole buffer, a `PipeData` with everything kept, and a `List` entry costs exactly `MAX_LIST_ENTRY`.
#[test]
fn replies_that_name_an_agent_fit_a_frame_with_any_id_the_holder_takes() {
    assert!(agent_id_fits(&format!("agent-{}", u64::MAX)));
    assert!(agent_id_fits(&"x".repeat(MAX_AGENT_ID)));
    assert!(!agent_id_fits(&"x".repeat(MAX_AGENT_ID + 1)));
    assert!(!agent_id_fits(""));
    let info = AgentInfo {
        agent: "x".repeat(MAX_AGENT_ID),
        pid: u32::MAX,
        state: AgentState::Exited {
            code: Some(i32::MIN),
            signal: Some(i32::MAX),
        },
    };
    assert_eq!(
        Reply::Agents(vec![info.clone()]).encode().len(),
        1 + 4 + MAX_LIST_ENTRY
    );
    let terminal = Reply::Terminal(TerminalInfo {
        info: info.clone(),
        buffer_dropped: true,
        buffer: vec![b'o'; agora_platform::HELD_BUFFER],
    });
    assert!(terminal.encode().len() <= MAX_FRAME);
    let pipe = Reply::PipeData(PipeData {
        info,
        dropped: u64::MAX,
        eof: true,
        seen: Vec::new(),
        fresh: vec![vec![b'o'; agora_platform::HELD_LINE_MAX]],
    });
    assert!(pipe.encode().len() <= MAX_FRAME);
}
