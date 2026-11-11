//! Saved task graphs and live leases survive a join that is the first board operation after reopening the hub.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use agora_hub::{
    board::{Boards, Verdict},
    hub::Hub,
    projection::View,
    store::{EventStore, SqliteStore},
};
use agora_protocol::v1::{self as v1, event::Body};
use prost::Message;
use serde_json::{json, Value};

fn events() -> Vec<v1::Event> {
    let mut events = Vec::new();
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
            events.push(v1::Event {
                producer_id: node.into(),
                seq: index as u64 + 1,
                at_ms: 1000,
                body: Some(body),
            });
        }
    }
    events
}

fn result(verdict: Verdict) -> (bool, Value) {
    match verdict {
        Verdict::Done { result, is_error } => (
            is_error,
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap(),
        ),
        other => panic!("unexpected verdict: {other:?}"),
    }
}

fn call(hub: &Hub, node: &str, tool: &str, args: Value) -> (bool, Value) {
    result(
        hub.board_call(
            node,
            &v1::BoardCall {
                from_agent: "a".into(),
                project_id: "p".into(),
                tool: tool.into(),
                arguments_json: args.to_string(),
                ..Default::default()
            },
            2001,
        )
        .verdict,
    )
}

#[test]
fn joins_preserve_saved_tasks_dependencies_and_live_leases_without_reloading_initialized_boards() {
    for preload in [false, true] {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut view = View::default();
        for event in events() {
            view.apply(&event);
            store
                .append(
                    &event.producer_id,
                    event.seq,
                    event.at_ms,
                    &event.encode_to_vec(),
                )
                .unwrap();
        }
        let old = Boards::new();
        let (failed, root) = result(
            old.call(
                &view,
                "alpha",
                "a",
                "p",
                "task_create",
                &json!({"title":"saved prerequisite", "depends_on":[]}).to_string(),
                1000,
            )
            .verdict,
        );
        assert!(!failed);
        let (_, dependent) = result(
            old.call(
                &view,
                "alpha",
                "a",
                "p",
                "task_create",
                &json!({"title":"saved dependent", "depends_on":[root["task_id"]]}).to_string(),
                1000,
            )
            .verdict,
        );
        let (failed, old_lease) = result(
            old.call(
                &view,
                "alpha",
                "a",
                "p",
                "lease_acquire",
                &json!({"paths":["README.md"], "ttl_s":120}).to_string(),
                1000,
            )
            .verdict,
        );
        assert!(!failed);
        let saved_before_reopen = old.snapshot("p", 1000).unwrap();
        store.set_setting("board.p", &saved_before_reopen).unwrap();
        let hub = Hub::open(store).unwrap();
        if preload {
            assert_eq!(
                call(&hub, "alpha", "task_list", json!({}))
                    .1
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            assert!(
                !call(
                    &hub,
                    "alpha",
                    "task_create",
                    json!({"title":"newer in-memory task", "depends_on":[]})
                )
                .0
            );
            assert!(
                !call(
                    &hub,
                    "alpha",
                    "task_claim",
                    json!({"task_id":root["task_id"]})
                )
                .0
            );
            // Storage can lag after a best-effort write failure. Joining must keep the already-loaded claim.
            hub.store()
                .set_setting("board.p", &saved_before_reopen)
                .unwrap();
        }
        let notices = hub.board_joined(
            "beta",
            "p",
            &v1::BoardJoinResult {
                tasks: vec![v1::BoardJoinTask {
                    task_id: "local-1".into(),
                    title: "incoming task".into(),
                    creator_agent_id: "a".into(),
                    ..Default::default()
                }],
                leases: vec![v1::BoardJoinLease {
                    agent_id: "a".into(),
                    path: "README.md".into(),
                    expires_at_ms: 6000,
                }],
            },
            2001,
        );
        let (failed, tasks) = call(&hub, "alpha", "task_list", json!({}));
        assert!(!failed);
        let tasks = tasks.as_array().unwrap();
        assert_eq!(
            tasks.len(),
            if preload { 4 } else { 3 },
            "preload={preload}: {tasks:?}"
        );
        assert!(tasks.iter().any(
            |task| task["task_id"] == root["task_id"] && task["title"] == "saved prerequisite"
        ));
        let saved_root = tasks
            .iter()
            .find(|task| task["task_id"] == root["task_id"])
            .unwrap();
        assert_eq!(
            saved_root["status"],
            if preload { "claimed" } else { "open" }
        );
        if preload {
            assert_eq!(saved_root["owner"], "alpha/a");
        }
        let saved_dependent = tasks
            .iter()
            .find(|task| task["task_id"] == dependent["task_id"])
            .unwrap();
        assert_eq!(saved_dependent["depends_on"], json!([root["task_id"]]));
        assert!(tasks.iter().any(|task| task["title"] == "incoming task"
            && task["task_id"] != root["task_id"]
            && task["task_id"] != dependent["task_id"]));
        let (failed, blocked) = call(
            &hub,
            "beta",
            "task_claim",
            json!({"task_id":dependent["task_id"]}),
        );
        assert!(failed);
        assert_eq!(blocked["error"]["code"], "task.blocked");
        let (failed, conflict) = call(
            &hub,
            "beta",
            "lease_acquire",
            json!({"paths":["README.md"], "ttl_s":120}),
        );
        assert!(failed);
        assert_eq!(conflict["error"]["code"], "lease.held");
        assert_eq!(conflict["error"]["params"]["holder_agent_id"], "alpha/a");
        assert_eq!(
            conflict["error"]["params"]["expires_at"],
            old_lease["expires_at"]
        );
        assert!(notices.iter().any(|notice| notice.to_node == "beta"
            && serde_json::from_str::<Value>(&notice.body).unwrap()["board"] == "lease_refused"));
        let saved: Value =
            serde_json::from_slice(&hub.store().setting("board.p").unwrap().unwrap()).unwrap();
        assert_eq!(saved["tasks"].as_array().unwrap().len(), tasks.len());
        assert!(saved["leases"]
            .as_array()
            .unwrap()
            .iter()
            .any(|lease| lease["holder"] == "alpha/a" && lease["path"] == "README.md"));
    }
}
