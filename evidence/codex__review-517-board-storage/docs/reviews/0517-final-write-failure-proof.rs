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
fn review_final_writes_rollback_events_snapshot_sequence_and_allow_retry() {
    for failure in ["snapshot_insert", "snapshot_update", "producer_update"] {
        let (_scratch, file, hub) = fixture(failure);
        if failure != "snapshot_insert" {
            ok(call(
                &hub,
                "alpha",
                "task_create",
                json!({"title":"seed","depends_on":[]}),
            ));
        }
        let before = hub.view();
        let seq = hub.store().acked("hub").unwrap();
        let saved = hub.store().setting("board.p").unwrap();
        let sub = hub.subscribe(Some(before.1)).unwrap();
        let conn = rusqlite::Connection::open(&file).unwrap();
        let trigger=match failure {
   "snapshot_insert"=>"CREATE TRIGGER review_final_write BEFORE INSERT ON settings WHEN NEW.key='board.p' BEGIN SELECT RAISE(ABORT,'review snapshot insert'); END;",
   "snapshot_update"=>"CREATE TRIGGER review_final_write BEFORE UPDATE ON settings WHEN NEW.key='board.p' BEGIN SELECT RAISE(ABORT,'review snapshot update'); END;",
   _=>"CREATE TRIGGER review_final_write BEFORE UPDATE ON producers WHEN NEW.producer='hub' BEGIN SELECT RAISE(ABORT,'review producer update'); END;",
  };
        conn.execute_batch(trigger).unwrap();
        let args = json!({"paths":["src/a.rs","src/b.rs"],"ttl_s":60,"reason":"review"});
        let refused = call(&hub, "alpha", "lease_acquire", args.clone());
        assert!(
            matches!(refused,Verdict::Invalid(ref e) if e.code.as_str()=="internal"),
            "{failure}: {refused:?}"
        );
        assert_eq!(
            hub.view(),
            before,
            "{failure}: view or log position advanced"
        );
        assert_eq!(
            hub.store().acked("hub").unwrap(),
            seq,
            "{failure}: producer sequence advanced"
        );
        assert_eq!(
            hub.store().setting("board.p").unwrap(),
            saved,
            "{failure}: snapshot changed"
        );
        assert!(
            hub.store().events_after(before.1, 100).unwrap().is_empty(),
            "{failure}: events committed"
        );
        assert!(sub.live.try_recv().is_err(), "{failure}: event published");
        conn.execute_batch("DROP TRIGGER review_final_write")
            .unwrap();
        assert_eq!(
            ok(call(&hub, "alpha", "workspace_status", json!({})))["leases"],
            json!([])
        );
        // The same call now commits exactly once, reusing the rolled-back event sequence and log positions.
        ok(call(&hub, "alpha", "lease_acquire", args));
        assert_eq!(hub.store().acked("hub").unwrap(), seq + 2);
        let events: Vec<_> = sub.live.try_iter().collect();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events.iter().map(|e| e.event.seq).collect::<Vec<_>>(),
            [seq + 1, seq + 2]
        );
        assert_eq!(
            events.iter().map(|e| e.pos).collect::<Vec<_>>(),
            [before.1 + 1, before.1 + 2]
        );
        assert_eq!(hub.view().0.leases.len(), 2);
        eprintln!("{failure}: snapshot/events/sequence/view/live unchanged after failure; retry commits two grants with original sequence and positions");
        drop(conn);
        drop(hub);
        let reopened = open(&file);
        let leases = ok(call(&reopened, "alpha", "workspace_status", json!({})));
        assert_eq!(
            leases["leases"].as_array().unwrap().len(),
            2,
            "{failure}: successful retry not retained"
        );
    }
}
