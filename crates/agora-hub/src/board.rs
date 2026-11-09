//! The shared board of a project (ADR-0039): tasks, path leases and handoffs for a project that has checkouts on two or
//! more online nodes. The hub is the single writer, so a claim or a lease is decided in one place.
//!
//! A board is the same broker a node runs ([`agora_mesh::broker::Mesh`]) with agents named `<node>/<agent>`, so there is one
//! implementation of the rules. The board's agents are not kept by hand: before each call they are made to match the hub's
//! view (running agents of that project), so an agent that ended gives its tasks and leases back.

use std::collections::BTreeMap;
use std::sync::Mutex;

use agora_core::AgentId;
use agora_mesh::broker::{AgentInfo, Limits, Mesh};
use agora_mesh::mcp::{board_tool, BOARD_TOOLS};
use agora_protocol::v1::event::Body;
use agora_protocol::{Error, ErrorCode};
use serde_json::{json, Value};

use crate::projection::View;

/// A handoff the hub must now pass to the receiving agent's node, after the board accepted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    pub to_node: String,
    pub to_agent: String,
    pub from_node: String,
    pub from_agent: String,
    pub body: String,
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// The project is not on two online nodes: nothing was done; the node runs the tool on its own broker.
    NotShared,
    /// The tool ran on the board: its MCP result and whether it is a tool error.
    Done { result: Value, is_error: bool },
    /// The call itself is malformed (unknown tool, bad arguments).
    Invalid(Error),
}

#[derive(Debug)]
pub struct Answer {
    pub verdict: Verdict,
    /// Audit events the call produced, to be published as hub events.
    pub events: Vec<Body>,
    pub handoff: Option<Handoff>,
}

fn answer(verdict: Verdict) -> Answer {
    Answer {
        verdict,
        events: Vec::new(),
        handoff: None,
    }
}

/// The online nodes that have a checkout of `project`.
pub fn nodes_with(view: &View, project: &str) -> Vec<String> {
    let mut nodes: Vec<String> = view
        .checkouts
        .values()
        .filter(|(_, p, _)| p == project)
        .map(|(n, _, _)| n.clone())
        .filter(|n| view.nodes.get(n).is_some_and(|v| v.online))
        .collect();
    nodes.sort();
    nodes.dedup();
    nodes
}

#[derive(Default)]
pub struct Boards {
    by_project: Mutex<BTreeMap<String, Mesh>>,
}

fn tool_error(code: ErrorCode, key: &str, value: &str) -> Value {
    json!({
        "content": [{"type": "text", "text": json!({"error": {"code": code.as_str(), "params": {key: value}}}).to_string()}],
        "isError": true,
    })
}

impl Boards {
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes the board's agents match the running agents of the project in `view`.
    fn sync(mesh: &mut Mesh, view: &View, project: &str, now_ms: u64) {
        let wanted: BTreeMap<&String, &crate::projection::AgentView> = view
            .agents
            .iter()
            .filter(|(_, a)| a.project_id == project && a.running)
            .collect();
        for (key, a) in &wanted {
            let id = AgentId::new(key.as_str());
            if !mesh.agent_ids().contains(&id) {
                mesh.register(AgentInfo {
                    id,
                    kind: a.kind.clone(),
                    node: a.node_id.clone(),
                    project: project.to_string(),
                    label: key.to_string(),
                    worktree: String::new(),
                    state: "idle".into(),
                });
            }
        }
        for id in mesh.agent_ids() {
            if !wanted.contains_key(&id.as_str().to_string()) {
                mesh.unregister(&id, now_ms);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    /// Runs one board tool for `agent` of `node` (both as the hub knows them: `node` from the connection's certificate).
    pub fn call(
        &self,
        view: &View,
        node: &str,
        agent: &str,
        project: &str,
        tool: &str,
        arguments_json: &str,
        now_ms: u64,
    ) -> Answer {
        let bad = |field: &str| {
            answer(Verdict::Invalid(
                Error::new(ErrorCode::ProtocolInvalidMessage).with("field", field),
            ))
        };
        if !BOARD_TOOLS.contains(&tool) {
            return bad("tool");
        }
        let Ok(mut args) = serde_json::from_str::<Value>(arguments_json) else {
            return bad("arguments_json");
        };
        if !args.is_object() {
            return bad("arguments_json");
        }
        let nodes = nodes_with(view, project);
        // A node can only speak for a project it has a checkout of, and a board exists only when two nodes share it.
        if nodes.len() < 2 || !nodes.iter().any(|n| n == node) {
            return answer(Verdict::NotShared);
        }
        let caller = format!("{node}/{agent}");
        let mut guard = self.by_project.lock().unwrap_or_else(|e| e.into_inner());
        let mesh = guard
            .entry(project.to_string())
            .or_insert_with(|| Mesh::new(Limits::default()));
        Self::sync(mesh, view, project, now_ms);
        mesh.expire_leases(now_ms);

        let mut target: Option<(String, String)> = None;
        if tool == "handoff" {
            let Some(to) = args.get("to").and_then(Value::as_str) else {
                return bad("to");
            };
            // A bare name is an agent on the caller's own node.
            let qualified = if to.contains('/') {
                to.to_string()
            } else {
                format!("{node}/{to}")
            };
            if let Some((n, a)) = qualified.split_once('/') {
                target = Some((n.to_string(), a.to_string()));
            }
            // Fail before anything changes: a handoff to a node that cannot take the message must not move the task.
            if let Some((n, _)) = &target {
                if !view.nodes.get(n).is_some_and(|v| v.online) {
                    let r = tool_error(ErrorCode::NodeOffline, "node_id", n);
                    return answer(Verdict::Done {
                        result: r,
                        is_error: true,
                    });
                }
            }
            args["to"] = json!(qualified);
        }

        let Some(result) = board_tool(&AgentId::new(&caller), tool, &args, mesh, now_ms) else {
            return bad("arguments_json");
        };
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let events = mesh.take_events();
        let handoff = match (&target, is_error, tool) {
            (Some((to_node, to_agent)), false, "handoff") => Some(Handoff {
                to_node: to_node.clone(),
                to_agent: to_agent.clone(),
                from_node: node.to_string(),
                from_agent: agent.to_string(),
                body: args
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                refs: args
                    .get("refs")
                    .and_then(Value::as_array)
                    .map(|r| {
                        r.iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
            }),
            _ => None,
        };
        Answer {
            verdict: Verdict::Done { result, is_error },
            events,
            handoff,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::projection::{AgentView, NodeView};
    use agora_protocol::v1::AgentState;

    fn view(online: &[&str]) -> View {
        let mut v = View::default();
        for (i, n) in ["alpha", "beta", "gamma"].iter().enumerate() {
            v.nodes.insert(
                (*n).to_string(),
                NodeView {
                    hostname: (*n).into(),
                    os: String::new(),
                    build: String::new(),
                    online: online.contains(n),
                    agents_running: 1,
                    terminals_open: 0,
                    memory_used_bytes: 0,
                    last_event_ms: 0,
                },
            );
            v.checkouts
                .insert(format!("c{i}"), ((*n).into(), "p".into(), "/p".into()));
            v.agents.insert(
                format!("{n}/a"),
                AgentView {
                    node_id: (*n).into(),
                    project_id: "p".into(),
                    worktree_id: String::new(),
                    kind: "shell".into(),
                    state: AgentState::Idle,
                    reported: false,
                    session_id: None,
                    running: true,
                    exit_code: None,
                    input_tokens: 0,
                    output_tokens: 0,
                    model: String::new(),
                },
            );
        }
        v
    }

    fn run(b: &Boards, v: &View, node: &str, tool: &str, args: Value, now: u64) -> Answer {
        b.call(v, node, "a", "p", tool, &args.to_string(), now)
    }

    fn code(a: &Answer) -> String {
        let Verdict::Done { result, .. } = &a.verdict else {
            panic!("{:?}", a.verdict)
        };
        let text = result["content"][0]["text"].as_str().unwrap();
        serde_json::from_str::<Value>(text).unwrap()["error"]["code"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }

    #[test]
    fn one_online_node_is_not_a_shared_project_and_a_node_cannot_use_a_project_it_has_no_checkout_of(
    ) {
        let b = Boards::new();
        let v = view(&["alpha"]);
        let a = run(&b, &v, "alpha", "task_list", json!({}), 1);
        assert_eq!(a.verdict, Verdict::NotShared);
        let v = view(&["alpha", "beta"]);
        let a = b.call(&v, "gamma", "a", "p", "task_list", "{}", 1);
        assert_eq!(a.verdict, Verdict::NotShared);
        let a = b.call(&v, "alpha", "a", "p", "send_message", "{}", 1);
        assert!(matches!(a.verdict, Verdict::Invalid(_)));
        let a = b.call(&v, "alpha", "a", "p", "task_list", "[1]", 1);
        assert!(matches!(a.verdict, Verdict::Invalid(_)));
    }

    #[test]
    fn a_lease_ends_at_its_ttl_even_if_nobody_releases_it() {
        let b = Boards::new();
        let v = view(&["alpha", "beta"]);
        let args = json!({"paths": ["x"], "ttl_s": 10});
        assert_ne!(
            code(&run(&b, &v, "alpha", "lease_acquire", args.clone(), 1_000)),
            "lease.held"
        );
        assert_eq!(
            code(&run(&b, &v, "beta", "lease_acquire", args.clone(), 5_000)),
            "lease.held"
        );
        let a = run(&b, &v, "beta", "lease_acquire", args, 12_000);
        assert_eq!(code(&a), "");
        assert!(a.events.iter().any(|e| matches!(e, Body::LeaseReleased(_))));
    }

    #[test]
    fn an_agent_that_ended_gives_its_task_back() {
        let b = Boards::new();
        let mut v = view(&["alpha", "beta"]);
        let made = run(
            &b,
            &v,
            "alpha",
            "task_create",
            json!({"title": "t", "depends_on": []}),
            1,
        );
        let Verdict::Done { result, .. } = made.verdict else {
            panic!()
        };
        let id = agora_mesh::mcp::tool_result_value(&result).unwrap()["task_id"]
            .as_str()
            .unwrap()
            .to_string();
        run(&b, &v, "alpha", "task_claim", json!({"task_id": id}), 2);
        v.agents.get_mut("alpha/a").unwrap().running = false;
        let a = run(&b, &v, "beta", "task_claim", json!({"task_id": id}), 3);
        assert_eq!(code(&a), "");
    }

    #[test]
    fn a_handoff_to_an_offline_node_changes_nothing() {
        let b = Boards::new();
        let v = view(&["alpha", "beta"]);
        let made = run(
            &b,
            &v,
            "alpha",
            "task_create",
            json!({"title": "t", "depends_on": []}),
            1,
        );
        let Verdict::Done { result, .. } = made.verdict else {
            panic!()
        };
        let id = agora_mesh::mcp::tool_result_value(&result).unwrap()["task_id"]
            .as_str()
            .unwrap()
            .to_string();
        run(&b, &v, "alpha", "task_claim", json!({"task_id": id}), 2);
        // gamma has a checkout and an agent but is offline.
        let a = run(
            &b,
            &v,
            "alpha",
            "handoff",
            json!({"to": "gamma/a", "task_id": id, "summary": "s", "refs": []}),
            3,
        );
        assert_eq!(code(&a), "node.offline");
        assert!(a.handoff.is_none());
        // Still alpha's: beta cannot take it.
        assert_eq!(
            code(&run(
                &b,
                &v,
                "beta",
                "task_claim",
                json!({"task_id": id}),
                4
            )),
            "task.not_open"
        );
    }
}
