use crate::{
    projection::is_forged,
    session::{NodeSession, SessionError},
    store::{EventStore, SqliteStore},
};
use agora_protocol::{
    handshake::Support,
    v1::{self, event::Body},
    ErrorCode,
};
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
fn review629_fixed_classifier_matches_live_session_for_all_33_bodies() {
    let support = Support {
        generations: vec![1],
        capabilities: vec![],
        required: vec![],
    };
    let bodies = vec![
        Body::NodeOnline(v1::NodeOnline::decode(fixture("NodeOnline").as_slice()).unwrap()),
        Body::NodeOffline(v1::NodeOffline::decode(fixture("NodeOffline").as_slice()).unwrap()),
        Body::NodeResources(
            v1::NodeResources::decode(fixture("NodeResources").as_slice()).unwrap(),
        ),
        Body::NodeLink(v1::NodeLink::decode(fixture("NodeLink").as_slice()).unwrap()),
        Body::NodeTrustedDevices(
            v1::NodeTrustedDevices::decode(fixture("NodeTrustedDevices").as_slice()).unwrap(),
        ),
        Body::NodeForgotten(
            v1::NodeForgotten::decode(fixture("NodeForgotten").as_slice()).unwrap(),
        ),
        Body::CheckoutDiscovered(
            v1::CheckoutDiscovered::decode(fixture("CheckoutDiscovered").as_slice()).unwrap(),
        ),
        Body::WorktreeCreated(
            v1::WorktreeCreated::decode(fixture("WorktreeCreated").as_slice()).unwrap(),
        ),
        Body::WorktreeRemoved(
            v1::WorktreeRemoved::decode(fixture("WorktreeRemoved").as_slice()).unwrap(),
        ),
        Body::GitStatusChanged(
            v1::GitStatusChanged::decode(fixture("GitStatusChanged").as_slice()).unwrap(),
        ),
        Body::WorkspaceRegistered(
            v1::WorkspaceRegistered::decode(fixture("WorkspaceRegistered").as_slice()).unwrap(),
        ),
        Body::WorkspaceStatusChanged(
            v1::WorkspaceStatusChanged::decode(fixture("WorkspaceStatusChanged").as_slice())
                .unwrap(),
        ),
        Body::AgentWorkspaceBound(
            v1::AgentWorkspaceBound::decode(fixture("AgentWorkspaceBound").as_slice()).unwrap(),
        ),
        Body::AgentStarted(v1::AgentStarted::decode(fixture("AgentStarted").as_slice()).unwrap()),
        Body::AgentStateChanged(
            v1::AgentStateChanged::decode(fixture("AgentStateChanged").as_slice()).unwrap(),
        ),
        Body::AgentSessionReported(
            v1::AgentSessionReported::decode(fixture("AgentSessionReported").as_slice()).unwrap(),
        ),
        Body::AgentUsageReported(
            v1::AgentUsageReported::decode(fixture("AgentUsageReported").as_slice()).unwrap(),
        ),
        Body::AgentExited(v1::AgentExited::decode(fixture("AgentExited").as_slice()).unwrap()),
        Body::AgentForgotten(
            v1::AgentForgotten::decode(fixture("AgentForgotten").as_slice()).unwrap(),
        ),
        Body::AgentSessionOptions(
            v1::AgentSessionOptions::decode(fixture("AgentSessionOptions").as_slice()).unwrap(),
        ),
        Body::AgentInterrupted(
            v1::AgentInterrupted::decode(fixture("AgentInterrupted").as_slice()).unwrap(),
        ),
        Body::TerminalOpened(
            v1::TerminalOpened::decode(fixture("TerminalOpened").as_slice()).unwrap(),
        ),
        Body::TerminalTitleChanged(
            v1::TerminalTitleChanged::decode(fixture("TerminalTitleChanged").as_slice()).unwrap(),
        ),
        Body::TerminalClosed(
            v1::TerminalClosed::decode(fixture("TerminalClosed").as_slice()).unwrap(),
        ),
        Body::ApprovalRequested(
            v1::ApprovalRequested::decode(fixture("ApprovalRequested").as_slice()).unwrap(),
        ),
        Body::ApprovalResolved(
            v1::ApprovalResolved::decode(fixture("ApprovalResolved").as_slice()).unwrap(),
        ),
        Body::MessagePosted(
            v1::MessagePosted::decode(fixture("MessagePosted").as_slice()).unwrap(),
        ),
        Body::TaskChanged(v1::TaskChanged::decode(fixture("TaskChanged").as_slice()).unwrap()),
        Body::LeaseGranted(v1::LeaseGranted::decode(fixture("LeaseGranted").as_slice()).unwrap()),
        Body::LeaseReleased(
            v1::LeaseReleased::decode(fixture("LeaseReleased").as_slice()).unwrap(),
        ),
        Body::HandoffCreated(
            v1::HandoffCreated::decode(fixture("HandoffCreated").as_slice()).unwrap(),
        ),
        Body::AgentStartRefused(
            v1::AgentStartRefused::decode(fixture("AgentStartRefused").as_slice()).unwrap(),
        ),
        Body::FilesChanged(v1::FilesChanged::decode(fixture("FilesChanged").as_slice()).unwrap()),
    ];
    let mut rejected = 0;
    for body in bodies {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut session = NodeSession::new(&store, "n1", &support);
        session
            .hello(
                &v1::Hello {
                    role: v1::Role::Node as i32,
                    protocol_generations: vec![1],
                    ..Default::default()
                },
                0,
            )
            .unwrap();
        session.resume(&v1::Resume::default()).unwrap();
        let mut ev = v1::Event {
            producer_id: "n1".into(),
            seq: 1,
            at_ms: 1,
            body: Some(body.clone()),
        };
        let forged = is_forged(&ev);
        let got = session.event(&ev.encode_to_vec());
        assert_eq!(
            matches!(&got,Err(SessionError::Protocol(e)) if e.code==ErrorCode::ProtocolInvalidMessage),
            forged,
            "{body:?}: {got:?}"
        );
        assert_eq!(store.acked("n1").unwrap(), u64::from(!forged));
        rejected += usize::from(forged);
        ev.producer_id = "hub".into();
        assert!(
            !is_forged(&ev),
            "legitimate hub event must not poison replay: {body:?}"
        );
    }
    assert_eq!(rejected, 10);
    println!("33 schema fixture bodies: live refusal matches classifier; 10 forbidden, 23 accepted; all hub-body controls trusted");
}
