#![allow(clippy::unwrap_used, clippy::expect_used)]
use agora_hub::{hub::Hub, server::snapshot_of, session::NodeSession, store::SqliteStore};
use agora_protocol::{
    handshake::Support,
    v1::{self as v1, event::Body},
};
use prost::Message;
#[test]
fn review_a_client_opened_while_forgotten_recovers_the_returning_machine() {
    let hub = Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap();
    let support = Support {
        generations: vec![1],
        capabilities: vec![],
        required: vec![],
    };
    let mut node = NodeSession::new(hub.store(), "n", &support);
    let hello = v1::Hello {
        role: v1::Role::Node as i32,
        protocol_generations: vec![1],
        ..Default::default()
    };
    node.hello(&hello, 1).unwrap();
    node.resume(&v1::Resume::default()).unwrap();
    let (reg, _cmds, gen) = hub.node_link_opened("n", vec![], 1);
    let bodies = [
        Body::NodeOnline(v1::NodeOnline {
            node_id: "n".into(),
            hostname: "laptop".into(),
            os: "linux".into(),
            build: "test".into(),
        }),
        Body::CheckoutDiscovered(v1::CheckoutDiscovered {
            node_id: "n".into(),
            checkout_id: "c".into(),
            project_id: "p".into(),
            path: "/project".into(),
        }),
        Body::WorkspaceRegistered(v1::WorkspaceRegistered {
            workspace_id: "ws".into(),
            node_id: "n".into(),
            project_id: "p".into(),
            path: "/project".into(),
            ..Default::default()
        }),
        Body::AgentStarted(v1::AgentStarted {
            agent_id: "a".into(),
            node_id: "n".into(),
            project_id: "p".into(),
            kind: "shell".into(),
            ..Default::default()
        }),
        Body::AgentWorkspaceBound(v1::AgentWorkspaceBound {
            agent_id: "a".into(),
            workspace_id: "ws".into(),
        }),
    ];
    for (i, body) in bodies.into_iter().enumerate() {
        let seq = i as u64 + 1;
        hub.ingest(
            &mut node,
            &v1::Event {
                producer_id: "n".into(),
                seq,
                at_ms: seq,
                body: Some(body),
            }
            .encode_to_vec(),
        )
        .unwrap();
    }
    hub.node_link_closed(reg, gen, false, 6, 6);
    hub.forget_node("n", false, 7).unwrap();
    let late = hub.subscribe(None).unwrap();
    let (view, pos) = late.snapshot.as_ref().unwrap();
    let hidden = snapshot_of(view, *pos);
    assert!(
        hidden.nodes.is_empty()
            && hidden.checkouts.is_empty()
            && hidden.workspaces.is_empty()
            && hidden.agents.is_empty()
    );
    let _back = hub.node_link_opened("n", vec![], 8);
    // A plain reconnect resumes at the acknowledged node sequence and has no missing announcement to replay.
    let mut returning = NodeSession::new(hub.store(), "n", &support);
    returning.hello(&hello, 8).unwrap();
    assert_eq!(
        returning.resume(&v1::Resume::default()).unwrap().from_seq,
        6
    );
    let events: Vec<_> = late.live.try_iter().collect();
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0].event.body, Some(Body::NodeLink(_))));
    let (current, pos) = hub.view();
    let fresh = snapshot_of(&current, pos);
    assert_eq!(fresh.agents.len(), 1);
    assert_eq!(fresh.checkouts.len(), 1);
    assert_eq!(fresh.workspaces.len(), 1);
    let out = std::path::Path::new("/tmp/agora-535-wire");
    std::fs::create_dir_all(out).unwrap();
    std::fs::write(out.join("hidden.snapshot.bin"), hidden.encode_to_vec()).unwrap();
    std::fs::write(out.join("returned.snapshot.bin"), fresh.encode_to_vec()).unwrap();
    std::fs::write(out.join("live.event.bin"), events[0].event.encode_to_vec()).unwrap();
    std::fs::write(out.join("position.txt"), events[0].pos.to_string()).unwrap();
    eprintln!("hidden snapshot: 0 nodes/folders/agents; live after reconnect: NodeLink only; fresh snapshot: 1 node, 1 checkout, 1 workspace, 1 running agent; node resumes at seq 6");
}
