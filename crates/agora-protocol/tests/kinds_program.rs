//! ADR-0069: `AgentKind.program`, `NodeOnline.binary` and `NodeSnapshot.binary` are additive. The published fixtures (written before
//! the fields existed, never edited) still decode, with the new fields empty, and the new fields round-trip.
#![allow(clippy::unwrap_used)]

use agora_protocol::v1;
use prost::Message;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/fixtures/v1")
            .join(format!("agora.v1.{name}.bin")),
    )
    .unwrap()
}

#[test]
fn old_bytes_decode_with_the_new_fields_empty() {
    let k = v1::AgentKind::decode(fixture("AgentKind").as_slice()).unwrap();
    assert_eq!((k.kind.as_str(), k.program.as_str()), ("x", ""));
    let n = v1::NodeOnline::decode(fixture("NodeOnline").as_slice()).unwrap();
    assert_eq!((n.build.as_str(), n.binary.as_str()), ("x", ""));
    let s = v1::NodeSnapshot::decode(fixture("NodeSnapshot").as_slice()).unwrap();
    assert_eq!((s.node_id.as_str(), s.binary.as_str()), ("x", ""));
}

#[test]
fn the_new_fields_round_trip_and_an_old_reader_skips_them() {
    let k = v1::AgentKind {
        kind: "wrongstack".into(),
        program: "wstack".into(),
        ..Default::default()
    };
    let back = v1::AgentKind::decode(k.encode_to_vec().as_slice()).unwrap();
    assert_eq!(back.program, "wstack");
    let n = v1::NodeOnline {
        node_id: "win".into(),
        binary: r"C:\Users\a\Downloads\agora.exe".into(),
        ..Default::default()
    };
    let bytes = n.encode_to_vec();
    assert_eq!(
        v1::NodeOnline::decode(bytes.as_slice()).unwrap().binary,
        n.binary
    );
    // A reader that knows only the first four fields (an old client) gets the same values for them: field 5 is skipped.
    #[derive(Clone, PartialEq, prost::Message)]
    struct OldNodeOnline {
        #[prost(string, tag = "1")]
        node_id: String,
        #[prost(string, tag = "4")]
        build: String,
    }
    assert_eq!(
        OldNodeOnline::decode(bytes.as_slice()).unwrap().node_id,
        "win"
    );
}
