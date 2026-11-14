//! A board change is answered as done only once it is stored (the architecture review's R2): when the hub cannot write it, the
//! agent gets an error, and the board, in memory and after a restart, is as it was before the call.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use agora_hub::{
    board::Verdict,
    hub::Hub,
    store::{EventStore, SqliteStore},
};
use agora_protocol::v1::{self as v1, event::Body};
use prost::Message;
use serde_json::{json, Value};
use std::path::Path;

/// Two online nodes with a checkout of project `p` and an agent `a` each: the project's board is shared, so the hub runs it.
fn shared_project(store: &SqliteStore) {
    for node in ["alpha", "beta"] {
        let bodies = [
            Body::NodeOnline(v1::NodeOnline {
                node_id: node.into(),
                ..Default::default()
            }),
            Body::CheckoutDiscovered(v1::CheckoutDiscovered {
                checkout_id: "c".into(),
                node_id: node.into(),
                project_id: "p".into(),
                path: "/project".into(),
            }),
            Body::AgentStarted(v1::AgentStarted {
                agent_id: "a".into(),
                node_id: node.into(),
                project_id: "p".into(),
                kind: "fixture".into(),
                ..Default::default()
            }),
        ];
        for (index, body) in bodies.into_iter().enumerate() {
            let event = v1::Event {
                producer_id: node.into(),
                seq: index as u64 + 1,
                at_ms: 1000,
                body: Some(body),
            };
            store
                .append(node, event.seq, event.at_ms, &event.encode_to_vec())
                .unwrap();
        }
    }
}

fn call(hub: &Hub, node: &str, tool: &str, args: Value) -> Verdict {
    hub.board_call(
        node,
        &v1::BoardCall {
            from_agent: "a".into(),
            project_id: "p".into(),
            tool: tool.into(),
            arguments_json: args.to_string(),
            ..Default::default()
        },
        2000,
    )
    .verdict
}

fn ok(v: Verdict) -> Value {
    match v {
        Verdict::Done {
            result,
            is_error: false,
        } => serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap(),
        other => panic!("expected a successful tool result, got {other:?}"),
    }
}

fn tasks(hub: &Hub) -> Vec<Value> {
    ok(call(hub, "alpha", "task_list", json!({})))
        .as_array()
        .unwrap()
        .clone()
}

/// Opens the hub on the file. A hub that opens has no connections and records both machines offline (ADR-0070); they redial.
fn open(file: &Path) -> Hub {
    let hub = Hub::open(SqliteStore::open(file).unwrap()).unwrap();
    for node in ["alpha", "beta"] {
        let _ = hub.node_link_opened(node, Vec::new(), 2000);
    }
    hub
}

#[test]
fn a_board_change_the_hub_cannot_store_is_refused_and_undone() {
    let dir = std::env::temp_dir().join(format!("agora-board-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("hub.db");
    {
        let store = SqliteStore::open(&file).unwrap();
        shared_project(&store);
    }
    let hub = open(&file);
    let created = ok(call(
        &hub,
        "alpha",
        "task_create",
        json!({"title": "fix login", "depends_on": []}),
    ));
    let task = created["task_id"].as_str().unwrap().to_string();
    let positions = hub.view().1;

    // Another writer holds the database: the hub's next write fails (after SQLite's busy wait).
    let blocker = rusqlite::Connection::open(&file).unwrap();
    blocker.execute_batch("BEGIN EXCLUSIVE;").unwrap();
    match call(&hub, "beta", "task_claim", json!({"task_id": task})) {
        Verdict::Invalid(e) => assert_eq!(e.code.as_str(), "internal", "{e:?}"),
        other => panic!("a claim that was not stored must not be answered as done: {other:?}"),
    }
    blocker.execute_batch("ROLLBACK;").unwrap();
    drop(blocker);

    // Nothing changed: not in memory, not in the log, and not after the hub opens the file again.
    let now = tasks(&hub);
    assert_eq!(now.len(), 1, "{now:?}");
    assert!(
        now[0]["owner"].is_null(),
        "the refused claim stayed in memory: {now:?}"
    );
    assert_eq!(
        hub.view().1,
        positions,
        "the refused claim's events reached the log"
    );
    drop(hub);
    let reopened = open(&file);
    let after = tasks(&reopened);
    assert_eq!(after.len(), 1, "{after:?}");
    assert!(
        after[0]["owner"].is_null(),
        "the refused claim came back after a restart: {after:?}"
    );

    // And the board works again once the store does.
    let claimed = ok(call(
        &reopened,
        "beta",
        "task_claim",
        json!({"task_id": task}),
    ));
    assert_eq!(claimed["ok"], true, "{claimed}");
    assert_eq!(tasks(&reopened)[0]["owner"], "beta/a");
    let _ = std::fs::remove_dir_all(&dir);
}
