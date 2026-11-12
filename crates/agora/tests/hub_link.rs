//! A node's events reach a hub's store and view through the two state machines (the node's `Uplink` and the hub's
//! `NodeSession`), with the messages passed in memory. Transport and framing are tested where they live; this is
//! the protocol between them: resume, replay, no duplicates, no gaps, a resync after lost events (ADR-0054), and two machines
//! in one view.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;

use agora_core::AgentId;
use agora_hub::projection::View;
use agora_hub::session::{Ingest, NodeSession};
use agora_hub::store::{EventStore, SqliteStore};
use agora_node::daemon::{Announce, EventSink};
use agora_node::uplink::{Replay, Uplink, UplinkError};
use agora_protocol::handshake::Support;
use agora_protocol::v1::AgentState;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agora-link-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn support() -> Support {
    Support {
        generations: vec![1],
        capabilities: vec!["terminal.raw".into()],
        required: vec![],
    }
}

/// A node with a checkout and one running agent: 6 events.
fn node(dir: &std::path::Path, id: &str) -> Arc<EventSink> {
    let sink = Arc::new(EventSink::open(dir, id).unwrap());
    sink.node_online(id);
    sink.checkout_discovered(id, "proj", std::path::Path::new("/work/proj"));
    sink.agent_launched(
        &AgentId::new(format!("agent-{id}")),
        "proj",
        id,
        std::path::Path::new("/work/proj"),
        std::path::Path::new("/work/wt"),
        "agent/x",
        "shell",
        true,
    );
    sink
}

/// One connection: handshake, then send up to `limit` events (None = everything), acking as the hub does.
/// Returns how many events the hub applied.
fn connect(
    store: &SqliteStore,
    sink: &Arc<EventSink>,
    node_id: &str,
    limit: Option<usize>,
) -> Result<usize, UplinkError> {
    let sup = support();
    let mut hub = NodeSession::new(store, node_id, &sup);
    let mut up = Uplink::new(Arc::clone(sink), "test", &["terminal.raw"]);

    let hello = up.hello()?;
    let welcome = hub.hello(&hello, 0).unwrap();
    up.on_welcome(&welcome)?;
    let ok = hub.resume(&up.resume()?).unwrap();
    // A node that no longer has what the hub asks for says so before sending more (ADR-0054), as `uplink_client` does.
    if let Replay::Resync { .. } = up.on_resume_ok(&ok)? {
        hub.snapshot_required(0).unwrap();
    }

    let mut applied = 0;
    'send: loop {
        let batch = up.next_batch(2)?;
        if batch.is_empty() {
            break;
        }
        for payload in batch {
            if limit.is_some_and(|l| applied >= l) {
                break 'send; // the connection drops here
            }
            match hub.event(&payload).unwrap() {
                Ingest::Applied { acked, .. } | Ingest::Resynced { acked, .. } => {
                    applied += 1;
                    up.on_ack(acked)?;
                }
                Ingest::Duplicate { acked } => up.on_ack(acked)?,
                Ingest::Gap { expected } => {
                    if let Replay::Resync { .. } = up.on_gap(expected)? {
                        hub.snapshot_required(0).unwrap();
                    }
                }
            }
        }
    }
    Ok(applied)
}

#[test]
fn all_events_arrive_in_order_and_the_node_trims_its_buffer() {
    let store = SqliteStore::open_in_memory().unwrap();
    let sink = node(&tmp("all"), "n1");
    let total = sink.next_seq() - 1;
    assert_eq!(connect(&store, &sink, "n1", None).unwrap() as u64, total);
    assert_eq!(store.acked("n1").unwrap(), total);
    assert_eq!(sink.acked(), total, "the node learned what the hub holds");
    // A second connection has nothing left to send.
    assert_eq!(connect(&store, &sink, "n1", None).unwrap(), 0);
}

#[test]
fn a_dropped_connection_resumes_without_duplicates_or_gaps() {
    let store = SqliteStore::open_in_memory().unwrap();
    let sink = node(&tmp("drop"), "n1");
    let total = sink.next_seq() - 1;
    assert_eq!(connect(&store, &sink, "n1", Some(3)).unwrap(), 3);
    // More happens while disconnected: nodes keep working without the hub.
    sink.node_resources("n1", 1, 1, 0);
    let rest = connect(&store, &sink, "n1", None).unwrap() as u64;
    assert_eq!(rest, total + 1 - 3);
    let seqs: Vec<u64> = store
        .events_after(0, 100)
        .unwrap()
        .iter()
        .map(|e| e.seq)
        .collect();
    assert_eq!(seqs, (1..=total + 1).collect::<Vec<_>>());
}

#[test]
fn a_hub_restored_from_an_older_backup_gets_the_gap_replayed() {
    let dir = tmp("restore");
    std::fs::create_dir_all(&dir).unwrap();
    let live = dir.join("hub.db");
    let sink = node(&dir.join("n"), "n1");
    {
        let store = SqliteStore::open(&live).unwrap();
        connect(&store, &sink, "n1", Some(2)).unwrap();
        store.backup_to(&dir.join("backup.db")).unwrap();
        // Enough happens after the backup that, once acknowledged, the node's buffer is compacted past it.
        let long = std::path::PathBuf::from(format!("/work/{}", "x".repeat(1000)));
        for _ in 0..80 {
            sink.checkout_discovered("n1", "proj", &long);
        }
        connect(&store, &sink, "n1", None).unwrap();
        assert_eq!(store.acked("n1").unwrap(), sink.next_seq() - 1);
    }
    // Disaster: the hub comes back from the backup, which only has 2 events.
    agora_hub::store::restore(&dir.join("backup.db"), &live).unwrap();
    let store = SqliteStore::open(&live).unwrap();
    assert_eq!(store.acked("n1").unwrap(), 2);
    let oldest = sink.oldest_retained();
    assert!(
        oldest > 3,
        "the node trimmed its buffer past 2 when the hub acked everything"
    );
    // The old events are gone from the node too: it resynchronizes (ADR-0054). Its present goes at the tail of its buffer, the hub
    // takes the oldest event it still has as the base and records the numbers before it as lost, and from then on everything
    // arrives. (This bare buffer has no announcer; agora-node's `a_resync_announces_the_nodes_present` covers what a real node says.)
    let delivered = connect(&store, &sink, "n1", None).unwrap() as u64;
    assert_eq!(delivered, sink.next_seq() - oldest);
    assert_eq!(store.acked("n1").unwrap(), sink.next_seq() - 1);
    assert_eq!(
        store.resyncs().unwrap(),
        [("n1".to_string(), 3, oldest - 1, 0)]
    );
    sink.node_resources("n1", 1, 1, 0);
    assert_eq!(connect(&store, &sink, "n1", None).unwrap(), 1);
    assert_eq!(store.resyncs().unwrap().len(), 1, "once");
}

/// F080 / ADR-0054: once the size cap dropped events the hub never acknowledged, the node used to end every connection in `Stale`
/// and the hub never heard from it again. Now it resynchronizes: the hub gets everything the node still has and all newer events,
/// and records the lost range.
#[test]
fn a_node_whose_cap_dropped_unacked_events_resyncs_and_delivers_its_newer_events() {
    let sink = Arc::new(EventSink::open_with_cap(&tmp("f080"), "n1", 1200).unwrap());
    for i in 0..5 {
        sink.node_resources("n1", i, i, 0);
    }
    let store = SqliteStore::open_in_memory().unwrap();
    assert_eq!(connect(&store, &sink, "n1", Some(3)).unwrap(), 3);
    // Offline for a while: the node keeps recording, and the cap drops events the hub never saw.
    for i in 0..60 {
        sink.node_resources("n1", 100 + i, i, 0);
    }
    let oldest = sink.oldest_retained();
    assert!(oldest > 4, "events 4.. were dropped unacknowledged");
    let delivered = connect(&store, &sink, "n1", None).unwrap();
    assert_eq!(delivered as u64, sink.next_seq() - oldest);
    assert_eq!(store.acked("n1").unwrap(), sink.next_seq() - 1);
    assert_eq!(
        store.resyncs().unwrap(),
        [("n1".to_string(), 4, oldest - 1, 0)]
    );
    // From then on the link is ordinary: newer events arrive, nothing more is skipped.
    sink.node_resources("n1", 1000, 0, 0);
    assert_eq!(connect(&store, &sink, "n1", None).unwrap(), 1);
    assert_eq!(store.acked("n1").unwrap(), sink.next_seq() - 1);
    assert_eq!(store.resyncs().unwrap().len(), 1);
}

/// A gap answer that arrives mid-session and asks for what the node no longer has resynchronizes too; a late second answer for
/// the same point (sent by the hub before it read `SnapshotRequired`) does not announce again.
#[test]
fn a_gap_answer_for_lost_events_resyncs_once_mid_session() {
    let dir = tmp("gap-resync");
    let sink = Arc::new(EventSink::open_with_cap(&dir, "n1", 600).unwrap());
    sink.set_announcer(Box::new(|ev, how| {
        if how == Announce::Restart {
            ev.node_online("n1");
        }
        ev.checkout_discovered("n1", "proj", std::path::Path::new("/work/proj"));
    }));
    sink.node_resources("n1", 0, 0, 0);
    let store = SqliteStore::open_in_memory().unwrap();
    connect(&store, &sink, "n1", None).unwrap();
    for i in 0..40 {
        sink.node_resources("n1", i, i, 0);
    }
    let oldest = sink.oldest_retained();
    assert!(oldest > 2);
    let sup = support();
    let mut hub = NodeSession::new(&store, "n1", &sup);
    let mut up = Uplink::new(Arc::clone(&sink), "test", &[]);
    let hello = up.hello().unwrap();
    up.on_welcome(&hub.hello(&hello, 0).unwrap()).unwrap();
    // The hub says 2 (its ack + 1); the node starts from what it has, as if it believed the hub were further on.
    let ok = hub.resume(&up.resume().unwrap()).unwrap();
    assert_eq!(ok.from_seq, 2);
    let start = agora_protocol::v1::ResumeOk { from_seq: oldest };
    assert_eq!(up.on_resume_ok(&start).unwrap(), Replay::Resumed);
    let first = up.next_batch(1).unwrap();
    assert_eq!(
        hub.event(&first[0]).unwrap(),
        Ingest::Gap { expected: 2 },
        "the hub wants 2"
    );
    // That gap answer asks for what is gone: the node resynchronizes, once.
    let before = sink.next_seq();
    assert_eq!(
        up.on_gap(2).unwrap(),
        Replay::Resync {
            from_seq: 2,
            // Read after the announcement, which may push more out under the cap.
            oldest_retained: sink.oldest_retained()
        }
    );
    let after_first = sink.next_seq();
    assert!(
        after_first > before,
        "the present was announced at the tail"
    );
    assert_eq!(up.on_gap(2).unwrap(), Replay::Resumed, "a late answer");
    assert_eq!(sink.next_seq(), after_first, "announced once");
    hub.snapshot_required(0).unwrap();
    for p in up.next_batch(1000).unwrap() {
        hub.event(&p).unwrap();
    }
    assert_eq!(store.acked("n1").unwrap(), sink.next_seq() - 1);
    // Only one event was in flight before the resync, so only one late answer could come; another means the hub ignored
    // `SnapshotRequired` (a hub from before ADR-0054), and the connection ends instead of looping.
    assert!(matches!(up.on_gap(2), Err(UplinkError::Stale { .. })));
}

#[test]
fn two_machines_show_up_in_one_view() {
    let store = SqliteStore::open_in_memory().unwrap();
    let (a, b) = (node(&tmp("ma"), "laptop"), node(&tmp("mb"), "server"));
    connect(&store, &a, "laptop", None).unwrap();
    connect(&store, &b, "server", None).unwrap();
    let (view, _) = View::rebuild(&store).unwrap();
    assert_eq!(view.nodes.len(), 2);
    assert!(view.nodes.values().all(|n| n.online));
    assert_eq!(view.agents.len(), 2);
    assert_eq!(view.worktrees.len(), 2);
    assert_eq!(view.terminals.len(), 2);
    assert_eq!(view.agents["laptop/agent-laptop"].node_id, "laptop");
    assert_eq!(view.agents["server/agent-server"].state, AgentState::Idle);
}

#[test]
fn messages_out_of_order_are_refused_on_the_node_side_too() {
    let sink = node(&tmp("order"), "n1");
    let mut up = Uplink::new(sink, "t", &[]);
    assert_eq!(up.next_batch(1).unwrap_err(), UplinkError::OutOfOrder);
    assert_eq!(up.resume().unwrap_err(), UplinkError::OutOfOrder);
    up.hello().unwrap();
    assert_eq!(up.hello().unwrap_err(), UplinkError::OutOfOrder);
    let bad = agora_protocol::v1::Welcome {
        chosen_generation: 9,
        ..Default::default()
    };
    assert_eq!(
        up.on_welcome(&bad).unwrap_err(),
        UplinkError::BadGeneration(9)
    );
}

// ADR-0055: a resync keeps a running agent's work on a shared board. A real node (agents are agora-fake-agent processes, the event
// buffer capped small) shares a project with a second machine through a hub, and both of its agents hold a file lease. Offline, one
// agent is stopped and the size cap drops that before the hub sees it, so the node resynchronizes when it comes back.

mod shared_board {
    use super::{support, tmp};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use agora_core::AgentId;
    use agora_hub::board::Verdict;
    use agora_hub::hub::Hub;
    use agora_hub::session::{Ingest, NodeSession};
    use agora_hub::store::SqliteStore;
    use agora_node::control::Client;
    use agora_node::daemon::{EventSink, Node, NodeConfig};
    use agora_node::uplink::{Replay, Uplink};
    use agora_protocol::v1::{self, command_request::Command as Cmd, event::Body};
    use prost::Message;
    use serde_json::{json, Value};

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    fn fake_agent() -> PathBuf {
        let mut fake = PathBuf::from(env!("CARGO_BIN_EXE_agora"));
        fake.pop();
        fake.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
        // Built once per test binary, even when present: a binary left from an older checkout may lack newer flags.
        static FRESH: std::sync::Once = std::sync::Once::new();
        FRESH.call_once(|| {
            assert!(std::process::Command::new(
                std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
            )
            .args(["build", "-q", "-p", "agora-fake-agent"])
            .status()
            .unwrap()
            .success());
        });
        fake
    }

    /// One connection of `sink` to `hub` through the hub's own ingest (so its boards see what arrives), until the node has nothing
    /// left to send. `each` runs after every event the hub applied.
    pub fn link(
        hub: &Hub,
        sink: &Arc<EventSink>,
        node: &str,
        mut each: impl FnMut(&Hub, &v1::Event),
    ) {
        let sup = support();
        let mut session = NodeSession::new(hub.store(), node, &sup);
        let mut up = Uplink::new(Arc::clone(sink), "test", &["terminal.raw"]);
        let hello = up.hello().unwrap();
        up.on_welcome(&session.hello(&hello, 0).unwrap()).unwrap();
        let ok = session.resume(&up.resume().unwrap()).unwrap();
        if let Replay::Resync { .. } = up.on_resume_ok(&ok).unwrap() {
            session.snapshot_required(0).unwrap();
        }
        loop {
            let batch = up.next_batch(1).unwrap();
            if batch.is_empty() {
                return;
            }
            for payload in batch {
                match hub.ingest(&mut session, &payload).unwrap() {
                    Ingest::Applied { acked, .. } | Ingest::Resynced { acked, .. } => {
                        up.on_ack(acked).unwrap();
                        each(hub, &v1::Event::decode(payload.as_slice()).unwrap());
                    }
                    Ingest::Duplicate { acked } => up.on_ack(acked).unwrap(),
                    Ingest::Gap { expected } => {
                        if let Replay::Resync { .. } = up.on_gap(expected).unwrap() {
                            session.snapshot_required(0).unwrap();
                        }
                    }
                }
            }
        }
    }

    /// A board tool call as `agent` of `node`: whether it failed, and the tool's JSON answer.
    pub fn board(
        hub: &Hub,
        node: &str,
        agent: &str,
        project: &str,
        tool: &str,
        args: Value,
    ) -> (bool, Value) {
        let call = v1::BoardCall {
            from_agent: agent.into(),
            project_id: project.into(),
            tool: tool.into(),
            arguments_json: args.to_string(),
            ..Default::default()
        };
        match hub
            .board_call(node, &call, agora_hub::server::now_ms())
            .verdict
        {
            Verdict::Done { result, is_error } => (
                is_error,
                serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap(),
            ),
            other => panic!("unexpected verdict: {other:?}"),
        }
    }

    pub fn owner(hub: &Hub, project: &str, task: &Value) -> Value {
        let (failed, tasks) = board(hub, "beta", "b", project, "task_list", json!({}));
        assert!(!failed, "{tasks}");
        tasks
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["task_id"] == task["task_id"])
            .map(|t| t["owner"].clone())
            .unwrap()
    }

    pub struct Scene {
        node: Option<Node>,
        dir: PathBuf,
        /// The hub, with the board in memory; the node has not reconnected since it lost events.
        pub hub: Hub,
        pub project: String,
        /// Hub-wide keys (`alpha/<id>`) and local ids of the agent that keeps running and the one that ended in the lost window.
        pub keeper: (String, String),
        pub quitter: (String, String),
        pub task: Value,
    }

    impl Scene {
        pub fn events(&self) -> &Arc<EventSink> {
            &self.node.as_ref().unwrap().events
        }
    }

    impl Drop for Scene {
        fn drop(&mut self) {
            self.node.take(); // stops its agents
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// See the comment above this module. `None` where this platform has no terminals or local sockets.
    pub fn scene(name: &str) -> Option<Scene> {
        if !agora_platform::PTY_SUPPORTED || !agora_platform::LOCAL_SOCKETS_SUPPORTED {
            return None;
        }
        let dir = tmp(name);
        let repo = dir.join("proj");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "x").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        let manifests = dir.join("manifests");
        std::fs::create_dir_all(&manifests).unwrap();
        let launch = serde_json::to_string(&vec![
            fake_agent().to_string_lossy().into_owned(),
            "--probe-directory".into(),
            dir.join("probe-{pid}.json").to_string_lossy().into_owned(),
        ])
        .unwrap();
        std::fs::write(
            manifests.join("probe.toml"),
            format!("kind = \"probe\"\ndisplay_name = \"Probe\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = {launch}\n"),
        )
        .unwrap();
        let cfg = NodeConfig {
            node_id: "alpha".into(),
            socket: dir.join("c.sock"),
            data_dir: dir.join("data"),
            worktree_root: dir.join("wt"),
            manifests_dir: manifests,
            projects: vec![repo],
            limits: Default::default(),
        };
        let node = Node::start_with_event_cap(&cfg, 16 * 1024).unwrap();
        let mut local = Client::connect(&cfg.socket).unwrap();
        let mut start = || {
            let ok = local
                .call(Cmd::AgentStart(v1::AgentStart {
                    kind: "probe".into(),
                    ..Default::default()
                }))
                .unwrap()
                .unwrap();
            let Some(v1::command_ok::Result::AgentStart(s)) = ok.result else {
                panic!("not started");
            };
            (format!("alpha/{}", s.agent_id), s.agent_id)
        };
        let (keeper, quitter) = (start(), start());
        let project = node
            .events
            .events_after(0)
            .into_iter()
            .find_map(|e| match e.body {
                Some(Body::CheckoutDiscovered(c)) => Some(c.project_id),
                _ => None,
            })
            .unwrap();

        // Another machine with the same project and an agent of its own.
        let beta = Arc::new(EventSink::open(&dir.join("beta"), "beta").unwrap());
        beta.node_online("beta");
        beta.checkout_discovered("beta", &project, Path::new("/elsewhere/proj"));
        beta.agent_started(&AgentId::new("b"), &project, "beta", "", "shell");

        let hub = Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap();
        link(&hub, &node.events, "alpha", |_, _| {});
        link(&hub, &beta, "beta", |_, _| {});
        let lease = |path: &str| json!({"paths": [path], "ttl_s": 3600});
        for (agent, path) in [(&keeper.1, "src/keep.rs"), (&quitter.1, "src/quit.rs")] {
            let (failed, answer) =
                board(&hub, "alpha", agent, &project, "lease_acquire", lease(path));
            assert!(!failed, "{answer}");
        }
        let (failed, task) = board(
            &hub,
            "beta",
            "b",
            &project,
            "task_create",
            json!({"title": "keep going", "depends_on": []}),
        );
        assert!(!failed, "{task}");
        let (failed, answer) = board(
            &hub,
            "alpha",
            &keeper.1,
            &project,
            "task_claim",
            json!({"task_id": task["task_id"]}),
        );
        assert!(!failed, "{answer}");
        assert_eq!(owner(&hub, &project, &task), keeper.0.as_str());

        // Offline: one agent is stopped, and so much is recorded that the size cap drops it before the hub ever sees it.
        let acked = node.events.acked();
        local
            .call(Cmd::AgentStop(v1::AgentStop {
                agent_id: quitter.1.clone(),
            }))
            .unwrap()
            .unwrap();
        let stopped_at = node.events.next_seq() - 1;
        let long = PathBuf::from(format!("/pad/{}", "x".repeat(1000)));
        for _ in 0..40 {
            node.events.checkout_discovered("alpha", "pad", &long);
        }
        assert!(node.events.oldest_retained() > stopped_at);
        assert!(
            node.events.oldest_retained() > acked + 1,
            "unacknowledged events were dropped"
        );
        assert!(
            hub.view().0.agents[&quitter.0].running,
            "the hub missed the stop"
        );
        Some(Scene {
            node: Some(node),
            dir,
            hub,
            project,
            keeper,
            quitter,
            task,
        })
    }
}

/// ADR-0055: after the resync the agent that still runs keeps its lease and its claimed task; the one that ended inside the lost
/// window shows as ended and its lease is released at the next board call.
#[test]
fn a_resync_keeps_a_running_agents_shared_board_work() {
    use shared_board::{board, link, owner, scene};
    let Some(s) = scene("board-keep") else {
        return;
    };
    link(&s.hub, s.events(), "alpha", |_, _| {});
    assert_eq!(
        s.hub.store().resyncs().unwrap().len(),
        1,
        "it resynchronized"
    );
    let (failed, answer) = board(
        &s.hub,
        "beta",
        "b",
        &s.project,
        "lease_acquire",
        serde_json::json!({"paths": ["src/keep.rs"], "ttl_s": 60}),
    );
    assert!(failed, "the running agent lost its lease: {answer}");
    assert_eq!(answer["error"]["code"], "lease.held");
    assert_eq!(
        answer["error"]["params"]["holder_agent_id"],
        s.keeper.0.as_str()
    );
    assert_eq!(owner(&s.hub, &s.project, &s.task), s.keeper.0.as_str());
    let view = s.hub.view().0;
    assert!(view.agents[&s.keeper.0].running);
    let ended = &view.agents[&s.quitter.0];
    assert!(!ended.running);
    assert_eq!(ended.exit_code, Some(i32::MIN), "stopped on request");
    let (failed, answer) = board(
        &s.hub,
        "beta",
        "b",
        &s.project,
        "lease_acquire",
        serde_json::json!({"paths": ["src/quit.rs"], "ttl_s": 60}),
    );
    assert!(!failed, "the ended agent's lease was released: {answer}");
}

/// ADR-0055: while the hub takes in the resync, a board call from the other machine after any of its events never sees the running
/// agent as ended, so never frees its work.
#[test]
fn board_calls_during_a_resync_never_free_a_running_agents_work() {
    use shared_board::{board, link, owner, scene};
    let Some(s) = scene("board-window") else {
        return;
    };
    let (keeper, project, task) = (s.keeper.0.clone(), s.project.clone(), s.task.clone());
    let mut seen = 0;
    link(&s.hub, s.events(), "alpha", |hub, ev| {
        seen += 1;
        assert!(
            hub.view().0.agents[&keeper].running,
            "shown as ended after {ev:?}"
        );
        assert_eq!(owner(hub, &project, &task), keeper.as_str(), "after {ev:?}");
    });
    assert!(seen > 0);
    let (failed, answer) = board(
        &s.hub,
        "beta",
        "b",
        &s.project,
        "lease_acquire",
        serde_json::json!({"paths": ["src/keep.rs"], "ttl_s": 60}),
    );
    assert!(failed, "{answer}");
    assert_eq!(
        answer["error"]["params"]["holder_agent_id"],
        keeper.as_str()
    );
}
