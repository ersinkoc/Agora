#![allow(clippy::unwrap_used, clippy::expect_used)]

use agora_hub::{
    board::{nodes_with, project_memberships, Boards, Verdict},
    projection::View,
};
use agora_protocol::v1::{self as v1, event::Body};
use serde_json::{json, Value};

fn apply(view: &mut View, node: &str, body: Body) {
    view.apply(&v1::Event {
        producer_id: node.into(),
        seq: 1,
        at_ms: 1000,
        body: Some(body),
    });
}

fn online(view: &mut View, node: &str) {
    apply(
        view,
        node,
        Body::NodeOnline(v1::NodeOnline {
            node_id: node.into(),
            hostname: "Same laptop".into(),
            ..Default::default()
        }),
    );
}

fn workspace(view: &mut View, node: &str, project: &str) {
    apply(
        view,
        node,
        Body::WorkspaceRegistered(v1::WorkspaceRegistered {
            workspace_id: "w".into(),
            node_id: node.into(),
            project_id: project.into(),
            path: "/chosen/project".into(),
            ..Default::default()
        }),
    );
    apply(
        view,
        node,
        Body::AgentStarted(v1::AgentStarted {
            agent_id: "a".into(),
            node_id: node.into(),
            project_id: project.into(),
            kind: "shell".into(),
            ..Default::default()
        }),
    );
}

fn call(
    board: &Boards,
    view: &View,
    node: &str,
    project: &str,
    tool: &str,
    args: Value,
) -> (bool, Value) {
    match board
        .call(view, node, "a", project, tool, &args.to_string(), 1000)
        .verdict
    {
        Verdict::Done { result, is_error } => (
            is_error,
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap(),
        ),
        other => panic!("unexpected verdict: {other:?}"),
    }
}

#[test]
fn selected_workspaces_share_tasks_and_refuse_a_second_path_lease() {
    let mut view = View::default();
    for node in ["alpha", "beta"] {
        online(&mut view, node);
        workspace(&mut view, node, "p");
    }
    assert!(view.checkouts.is_empty());
    assert_eq!(nodes_with(&view, "p"), ["alpha", "beta"]);
    let board = Boards::new();
    assert!(
        !call(
            &board,
            &view,
            "alpha",
            "p",
            "task_create",
            json!({"title":"shared task", "depends_on":[]})
        )
        .0
    );
    let (failed, tasks) = call(&board, &view, "beta", "p", "task_list", json!({}));
    assert!(!failed);
    assert_eq!(tasks[0]["title"], "shared task");
    assert!(
        !call(
            &board,
            &view,
            "alpha",
            "p",
            "lease_acquire",
            json!({"paths":["README.md"], "ttl_s":120})
        )
        .0
    );
    let (failed, error) = call(
        &board,
        &view,
        "beta",
        "p",
        "lease_acquire",
        json!({"paths":["README.md"], "ttl_s":120}),
    );
    assert!(failed);
    assert_eq!(error["error"]["code"], "lease.held");
    assert_eq!(error["error"]["params"]["holder_agent_id"], "alpha/a");
}

#[test]
fn mixed_checkout_and_workspace_membership_deduplicates_node_identity() {
    let mut view = View::default();
    online(&mut view, "alpha");
    online(&mut view, "beta");
    view.checkouts.insert(
        "alpha/c".into(),
        ("alpha".into(), "p".into(), "/old/project".into()),
    );
    workspace(&mut view, "alpha", "p");
    workspace(&mut view, "beta", "p");
    assert_eq!(nodes_with(&view, "p"), ["alpha", "beta"]);
    online(&mut view, "gamma");
    workspace(&mut view, "gamma", "different-id");
    assert_eq!(nodes_with(&view, "p"), ["alpha", "beta"]);
    assert_eq!(nodes_with(&view, "different-id"), ["gamma"]);
    assert_eq!(
        Boards::new()
            .call(&view, "gamma", "a", "p", "task_list", "{}", 1000)
            .verdict,
        Verdict::NotShared
    );
}

#[test]
fn offline_unknown_and_spoofed_nodes_do_not_make_a_single_node_project_shared() {
    let mut view = View::default();
    online(&mut view, "alpha");
    workspace(&mut view, "alpha", "p");
    workspace(&mut view, "unknown", "p");
    apply(
        &mut view,
        "alpha",
        Body::WorkspaceRegistered(v1::WorkspaceRegistered {
            workspace_id: "spoof".into(),
            node_id: "beta".into(),
            project_id: "p".into(),
            path: "/spoof".into(),
            ..Default::default()
        }),
    );
    assert_eq!(nodes_with(&view, "p"), ["alpha"]);
    assert_eq!(
        Boards::new()
            .call(&view, "alpha", "a", "p", "task_list", "{}", 1000)
            .verdict,
        Verdict::NotShared
    );
    online(&mut view, "beta");
    workspace(&mut view, "beta", "p");
    apply(
        &mut view,
        "beta",
        Body::NodeOffline(v1::NodeOffline {
            node_id: "beta".into(),
        }),
    );
    assert_eq!(nodes_with(&view, "p"), ["alpha"]);
    // Join discovery still needs the project after every member disconnects; filtering belongs to nodes_with.
    assert!(project_memberships(&view).any(|(node, project)| node == "beta" && project == "p"));
}

#[test]
fn changing_a_workspace_identity_stops_membership_in_the_previous_project() {
    let mut view = View::default();
    for node in ["alpha", "beta"] {
        online(&mut view, node);
        workspace(&mut view, node, "p");
    }
    workspace(&mut view, "beta", "other");
    assert_eq!(nodes_with(&view, "p"), ["alpha"]);
    assert_eq!(nodes_with(&view, "other"), ["beta"]);
}

#[test]
#[ignore = "manual membership projection benchmark"]
fn membership_projection_benchmark() {
    let mut view = View::default();
    for machine in 0..100 {
        let node = format!("n{machine}");
        online(&mut view, &node);
        view.checkouts.insert(
            format!("{node}/c"),
            (
                node.clone(),
                format!("p{}", machine * 5 % 50),
                "/repo".into(),
            ),
        );
        for directory in 0..5 {
            apply(
                &mut view,
                &node,
                Body::WorkspaceRegistered(v1::WorkspaceRegistered {
                    workspace_id: format!("w{directory}"),
                    node_id: node.clone(),
                    project_id: format!("p{}", (machine * 5 + directory) % 50),
                    path: format!("/selected/{directory}"),
                    ..Default::default()
                }),
            );
        }
    }
    let projects: Vec<String> = (0..50).map(|i| format!("p{i}")).collect();
    let mut samples = Vec::new();
    for iteration in 0..120 {
        let start = std::time::Instant::now();
        let total: usize = projects
            .iter()
            .map(|project| std::hint::black_box(nodes_with(&view, project)).len())
            .sum();
        let elapsed = start.elapsed();
        assert_eq!(total, 500);
        if iteration >= 20 {
            samples.push(elapsed);
        }
    }
    samples.sort();
    println!("Membership: 100 nodes, 500 selected workspaces, 100 legacy checkout facts; 50-project scan p95 {:?} (100 measured rounds)", samples[94]);
}
