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
        let _ = hub.node_link_opened(node, Vec::new(), 2000).unwrap();
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

// #517 review (Codex, docs/reviews/0517-board-atomicity-proof.rs on codex/review-517-board-storage): faults made with SQLite
// triggers. The board and its audit events must be stored together or not at all.
struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("agora-517-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture(name: &str) -> (Scratch, std::path::PathBuf, Hub) {
    let scratch = Scratch::new(name);
    let file = scratch.0.join("hub.db");
    {
        let store = SqliteStore::open(&file).unwrap();
        shared_project(&store);
    }
    let hub = open(&file);
    (scratch, file, hub)
}
#[test]
fn a_refused_two_path_lease_publishes_no_partial_grant() {
    let (_scratch, file, hub) = fixture("partial");
    ok(call(
        &hub,
        "alpha",
        "task_create",
        json!({"title":"seed","depends_on":[]}),
    ));
    let pos = hub.view().1;
    let sub = hub.subscribe(Some(pos)).unwrap();
    let conn = rusqlite::Connection::open(&file).unwrap();
    let seq = hub.store().acked("hub").unwrap();
    conn.execute_batch(&format!("CREATE TRIGGER review_fail_second_event BEFORE INSERT ON events WHEN NEW.producer='hub' AND NEW.seq={} BEGIN SELECT RAISE(ABORT,'review event failure'); END;",seq+2)).unwrap();
    let verdict = call(
        &hub,
        "alpha",
        "lease_acquire",
        json!({"paths":["src/a.rs","src/b.rs"],"ttl_s":60,"reason":"review"}),
    );
    assert!(
        matches!(verdict,Verdict::Invalid(ref e) if e.code.as_str()=="internal"),
        "{verdict:?}"
    );
    conn.execute_batch("DROP TRIGGER review_fail_second_event")
        .unwrap();
    let status = ok(call(&hub, "alpha", "workspace_status", json!({})));
    let projected = hub.view().0.leases;
    let delivered: Vec<_> = sub.live.try_iter().collect();
    eprintln!("refused_batch={verdict:?}; board_leases={}; projected_leases={projected:?}; delivered={delivered:?}; log_advance={}",status["leases"],hub.view().1-pos);
    assert_eq!(status["leases"], json!([]));
    // Verify the authoritative broker freed the path while the window still holds the refused grant.
    ok(call(
        &hub,
        "beta",
        "lease_acquire",
        json!({"paths":["src/a.rs"],"ttl_s":60,"reason":"retry"}),
    ));
    let double = hub.view().0.leases;
    eprintln!("projection_after_beta_acquires_same_path={double:?}");
    assert!(
        projected.is_empty(),
        "refused lease batch leaked a grant into the view and live subscribers"
    );
    assert!(
        delivered.is_empty(),
        "refused lease batch published audit events"
    );
}
#[test]
fn a_refused_claim_never_reappears_after_a_restart() {
    let (_scratch, file, hub) = fixture("compensation");
    let created = ok(call(
        &hub,
        "alpha",
        "task_create",
        json!({"title":"fix login","depends_on":[]}),
    ));
    let task = created["task_id"].as_str().unwrap().to_string();
    let conn = rusqlite::Connection::open(&file).unwrap();
    conn.execute_batch("CREATE TRIGGER review_fail_event BEFORE INSERT ON events WHEN NEW.producer='hub' BEGIN SELECT RAISE(ABORT,'review event failure'); END;
 CREATE TRIGGER review_fail_undo BEFORE UPDATE ON settings WHEN NEW.key='board.p' AND json_extract(CAST(NEW.value AS TEXT),'$.tasks[0].owner') IS NULL BEGIN SELECT RAISE(ABORT,'review compensation failure'); END;").unwrap();
    let verdict = call(&hub, "beta", "task_claim", json!({"task_id":task}));
    assert!(
        matches!(verdict,Verdict::Invalid(ref e) if e.code.as_str()=="internal"),
        "{verdict:?}"
    );
    let memory = tasks(&hub);
    let saved: Value =
        serde_json::from_slice(&hub.store().setting("board.p").unwrap().unwrap()).unwrap();
    assert!(memory[0]["owner"].is_null());
    conn.execute_batch("DROP TRIGGER review_fail_event; DROP TRIGGER review_fail_undo;")
        .unwrap();
    drop(conn);
    drop(hub);
    let reopened = open(&file);
    let after = tasks(&reopened);
    eprintln!("refused_claim={verdict:?}; memory_before_restart={memory:?}; saved_after_refusal={saved}; board_after_restart={after:?}; client_tasks={:?}",reopened.view().0.tasks);
    assert!(
        after[0]["owner"].is_null(),
        "a refused claim became owned after restarting the hub"
    );
}
