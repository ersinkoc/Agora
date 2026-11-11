#![allow(clippy::unwrap_used, clippy::expect_used)]

use agora_hub::{
    board::{nodes_with, Boards, Verdict},
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
                kind: "shell".into(),
                ..Default::default()
            }),
        ];
        for (i, body) in bodies.into_iter().enumerate() {
            events.push(v1::Event {
                producer_id: node.into(),
                seq: i as u64 + 1,
                at_ms: 1000,
                body: Some(body),
            });
        }
    }
    events
}

fn view() -> View {
    let mut view = View::default();
    for event in events() {
        view.apply(&event);
    }
    view
}

fn result(verdict: Verdict) -> Value {
    match verdict {
        Verdict::Done {
            result,
            is_error: false,
        } => serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap(),
        other => panic!("unexpected board verdict: {other:?}"),
    }
}

fn task(i: usize) -> v1::BoardJoinTask {
    v1::BoardJoinTask {
        task_id: format!("local-{i}"),
        title: format!("imported-{i}"),
        creator_agent_id: "a".into(),
        ..Default::default()
    }
}

#[test]
fn a_join_before_the_first_board_call_preserves_the_persisted_board() {
    let store = SqliteStore::open_in_memory().unwrap();
    for event in events() {
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
    result(
        old.call(
            &view(),
            "alpha",
            "a",
            "p",
            "task_create",
            &json!({"title":"already persisted", "depends_on":[]}).to_string(),
            1000,
        )
        .verdict,
    );
    store
        .set_setting("board.p", &old.snapshot("p", 1000).unwrap())
        .unwrap();
    let hub = Hub::open(store).unwrap();
    hub.board_joined(
        "alpha",
        "p",
        &v1::BoardJoinResult {
            tasks: vec![task(1)],
            ..Default::default()
        },
        2000,
    );
    let listed = result(
        hub.board_call(
            "alpha",
            &v1::BoardCall {
                from_agent: "a".into(),
                project_id: "p".into(),
                tool: "task_list".into(),
                arguments_json: "{}".into(),
                ..Default::default()
            },
            2001,
        )
        .verdict,
    );
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["title"] == "already persisted"),
        "persisted task lost after join: {listed}"
    );
}

#[test]
fn every_task_taken_from_a_node_has_a_preserved_destination() {
    let board = Boards::new();
    let work = v1::BoardJoinResult {
        tasks: (0..501).map(task).collect(),
        ..Default::default()
    };
    board.joined(&view(), "alpha", "p", &work, 2000);
    let listed = result(
        board
            .call(&view(), "alpha", "a", "p", "task_list", "{}", 2001)
            .verdict,
    );
    assert_eq!(
        listed.as_array().unwrap().len(),
        501,
        "the node has already removed all 501 tasks; the remainder has no destination"
    );
}

#[test]
fn two_selected_workspaces_are_recognized_without_startup_checkouts() {
    let mut selected = view();
    selected.checkouts.clear();
    for node in ["alpha", "beta"] {
        selected.workspaces.insert(
            format!("{node}/w"),
            v1::WorkspaceRegistered {
                workspace_id: format!("{node}/w"),
                node_id: node.into(),
                project_id: "p".into(),
                path: "/chosen/project".into(),
                git_root: "/chosen/project".into(),
                ..Default::default()
            },
        );
    }
    assert_eq!(nodes_with(&selected, "p"), ["alpha", "beta"]);
    assert_eq!(Boards::new().membership(&selected, 1000).len(), 2);
}
