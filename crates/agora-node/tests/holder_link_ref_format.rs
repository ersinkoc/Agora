//! Golden fixtures of the holder link's terminals inside the frame (M2.6, `docs/specs/holder-link.md`, "Terminals inside the frame").
//! As for the first fixtures (`holder_link_format.rs`): a fixture is the hex of one frame's payload, a published one is never edited,
//! only added. Set `AGORA_NEW_FIXTURES=1` to write the files that are missing (it never overwrites one).
//!
//! Where a terminal travels beside the frame (Unix, `SCM_RIGHTS`), these frames are never sent and a node refuses them as it refuses
//! an unknown tag; where it travels inside (Windows), they decode.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use agora_node::holder_link::{AgentInfo, AgentState, LinkError, Reply, TerminalInfo};

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

/// A reference as Windows makes one (a version byte and two handle values); the link only carries it.
fn reference() -> Vec<u8> {
    let mut r = vec![1u8];
    r.extend_from_slice(&0x12cu64.to_be_bytes());
    r.extend_from_slice(&0x130u64.to_be_bytes());
    r
}

fn cases() -> Vec<(&'static str, Reply)> {
    vec![
        (
            "started-ref",
            Reply::StartedRef {
                agent: "a1".into(),
                pid: 4242,
                terminal: reference(),
            },
        ),
        (
            "terminal-ref",
            Reply::TerminalRef {
                info: TerminalInfo {
                    info: AgentInfo {
                        agent: "a1".into(),
                        pid: 4242,
                        state: AgentState::Running,
                    },
                    buffer_dropped: false,
                    buffer: b"hi\r\n".to_vec(),
                },
                terminal: reference(),
            },
        ),
    ]
}

#[test]
fn terminals_inside_the_frame_keep_their_encoding() {
    for (name, reply) in cases() {
        let bytes = stored(name, &reply.encode());
        assert_eq!(reply.encode(), bytes, "{name}: encoding changed");
        if agora_platform::TERMINAL_IN_BAND {
            assert_eq!(Reply::decode(&bytes).expect(name), reply, "{name}");
        } else {
            assert!(
                matches!(Reply::decode(&bytes), Err(LinkError::Malformed(_))),
                "{name}: a node whose terminals travel beside the frame refuses it"
            );
        }
    }
}

#[test]
fn a_reference_past_its_bound_or_empty_is_refused() {
    for terminal in [vec![], vec![7u8; agora_platform::TERMINAL_REF_MAX + 1]] {
        let frame = Reply::StartedRef {
            agent: "a1".into(),
            pid: 1,
            terminal,
        }
        .encode();
        assert!(matches!(
            Reply::decode(&frame),
            Err(LinkError::Malformed(_))
        ));
    }
    if agora_platform::TERMINAL_IN_BAND {
        let at_bound = Reply::StartedRef {
            agent: "a1".into(),
            pid: 1,
            terminal: vec![7u8; agora_platform::TERMINAL_REF_MAX],
        };
        assert_eq!(Reply::decode(&at_bound.encode()).unwrap(), at_bound);
    }
}
