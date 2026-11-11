// Integration regressions for #281 plus #291: copy into agora-hub/tests only in the combined source tree.
#![allow(clippy::unwrap_used)]
use agora_hub::{board::Boards, projection::View};
use agora_protocol::v1::{self as v1, event::Body};

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
            ..Default::default()
        }),
    );
}

fn folder(view: &mut View, node: &str, project: &str) {
    apply(
        view,
        node,
        Body::WorkspaceRegistered(v1::WorkspaceRegistered {
            workspace_id: "w".into(),
            node_id: node.into(),
            project_id: project.into(),
            path: "/selected".into(),
            ..Default::default()
        }),
    );
}

fn running(view: &mut View, node: &str) {
    apply(
        view,
        node,
        Body::AgentStarted(v1::AgentStarted {
            agent_id: "a".into(),
            node_id: node.into(),
            project_id: "p".into(),
            kind: "fixture".into(),
            ..Default::default()
        }),
    );
}

fn selected() -> View {
    let mut view = View::default();
    for node in ["alpha", "beta"] {
        online(&mut view, node);
        folder(&mut view, node, "p");
        running(&mut view, node);
    }
    view
}

#[test]
fn selected_project_members_trigger_join_discovery() {
    let mut view = selected();
    assert!(view.checkouts.is_empty());
    // The still-running sessions keep p even after both folders have been re-associated.
    for node in ["alpha", "beta"] {
        folder(&mut view, node, "other");
    }
    let mut requests: Vec<_> = Boards::new()
        .membership(&view, 1000)
        .into_iter()
        .filter(|(_, project)| project == "p")
        .collect();
    requests.sort();
    assert_eq!(
        requests,
        [("alpha".into(), "p".into()), ("beta".into(), "p".into())]
    );
}

#[test]
fn a_project_with_no_remaining_facts_is_not_treated_as_still_shared_when_it_returns() {
    let mut view = selected();
    let board = Boards::new();
    for (node, project) in board.membership(&view, 1000) {
        board.join_settled(&node, &project);
    }
    for node in ["alpha", "beta"] {
        folder(&mut view, node, "other");
        apply(
            &mut view,
            node,
            Body::AgentExited(v1::AgentExited {
                agent_id: "a".into(),
                exit_code: 0,
            }),
        );
    }
    for (node, project) in board.membership(&view, 2000) {
        board.join_settled(&node, &project);
    }
    folder(&mut view, "alpha", "p");
    online(&mut view, "gamma");
    folder(&mut view, "gamma", "p");
    let mut requests: Vec<_> = board
        .membership(&view, 3000)
        .into_iter()
        .filter(|(_, project)| project == "p")
        .collect();
    requests.sort();
    assert_eq!(
        requests,
        [("alpha".into(), "p".into()), ("gamma".into(), "p".into())]
    );
}
