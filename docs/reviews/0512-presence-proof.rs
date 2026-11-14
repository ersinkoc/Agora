#![allow(clippy::unwrap_used, clippy::expect_used)]
use agora_hub::{
    hub::Hub,
    projection::View,
    store::{EventStore, SqliteStore},
};
use agora_protocol::v1::{self as v1, event::Body};
use prost::Message;

fn put(s: &SqliteStore, producer: &str, seq: u64, at_ms: u64, body: Body) {
    let e = v1::Event {
        producer_id: producer.into(),
        seq,
        at_ms,
        body: Some(body),
    };
    s.append(producer, seq, at_ms, &e.encode_to_vec()).unwrap();
}
fn populate(s: &SqliteStore) {
    put(
        s,
        "n",
        1,
        10,
        Body::NodeOnline(v1::NodeOnline {
            node_id: "n".into(),
            ..Default::default()
        }),
    );
    put(
        s,
        "hub",
        1,
        20,
        Body::NodeLink(v1::NodeLink {
            node_id: "n".into(),
            up: true,
            generation: 7,
            last_heard_ms: 20,
            reason: v1::NodeLinkReason::Opened as i32,
        }),
    );
    put(
        s,
        "n",
        2,
        30,
        Body::AgentStarted(v1::AgentStarted {
            node_id: "n".into(),
            agent_id: "a".into(),
            project_id: "p".into(),
            ..Default::default()
        }),
    );
    put(
        s,
        "n",
        3,
        40,
        Body::TerminalOpened(v1::TerminalOpened {
            terminal_id: "t".into(),
            ..Default::default()
        }),
    );
    put(
        s,
        "n",
        4,
        50,
        Body::ApprovalRequested(v1::ApprovalRequested {
            approval_id: "r".into(),
            agent_id: "a".into(),
            ..Default::default()
        }),
    );
    for seq in 5..=1105 {
        put(
            s,
            "n",
            seq,
            seq,
            Body::NodeResources(v1::NodeResources {
                node_id: "n".into(),
                ..Default::default()
            }),
        );
    }
}
#[test]
fn review_reopen_paged_log_preserves_everything_except_presence() {
    let store = SqliteStore::open_in_memory().unwrap();
    populate(&store);
    let (mut expected, before_pos) = View::rebuild(&store).unwrap();
    assert!(expected.agents["n/a"].running);
    assert!(!expected.terminals.is_empty());
    assert!(!expected.approvals.is_empty());
    expected.nodes.get_mut("n").unwrap().online = false;
    let hub = Hub::open_at(store, 9000).unwrap();
    assert_eq!(hub.view(), (expected.clone(), before_pos + 2));
    assert_eq!(hub.store().acked("hub").unwrap(), 3);
    let (registration, _commands, generation) = hub.node_link_opened("n", vec![], 9001);
    assert_eq!(generation, 8);
    expected.nodes.get_mut("n").unwrap().online = true;
    expected.nodes.get_mut("n").unwrap().link_generation = 8;
    expected.nodes.get_mut("n").unwrap().last_event_ms = 9001;
    assert_eq!(hub.view().0, expected);
    assert_eq!(View::rebuild(hub.store()).unwrap(), hub.view());
    hub.node_link_closed(registration, generation, false, 9001, 9002);
    assert!(hub.view().0.agents["n/a"].running);
    assert_eq!(hub.view().0.approvals, expected.approvals);
}
#[test]
fn review_store_failure_refuses_open_and_recovers_without_inventing_last_seen() {
    for fail_seq in [2, 3] {
        let dir =
            std::env::temp_dir().join(format!("agora-512-store-{}-{fail_seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hub.sqlite");
        let store = SqliteStore::open(&path).unwrap();
        populate(&store);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(&format!("CREATE TRIGGER review_fail BEFORE INSERT ON events WHEN NEW.producer = 'hub' AND NEW.seq = {fail_seq} BEGIN SELECT RAISE(ABORT, 'review injection'); END;")).unwrap();
        assert!(Hub::open_at(store, 9000).is_err());
        conn.execute_batch("DROP TRIGGER review_fail").unwrap();
        drop(conn);
        let hub = Hub::open_at(SqliteStore::open(&path).unwrap(), 10000).unwrap();
        let n = &hub.view().0.nodes["n"];
        assert!(!n.online);
        assert_eq!(n.last_event_ms, 1105);
        assert_eq!(n.link_generation, 7);
        assert!(hub.view().0.agents["n/a"].running);
        assert_eq!(View::rebuild(hub.store()).unwrap(), hub.view());
        drop(hub);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
