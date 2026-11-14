//! Agents know who else is here (docs/specs/mesh-tools.md, "Awareness"): a real node whose agents are `agora-fake-agent`
//! processes asks `workspace_status` on its own, through an in-process hub that knows a second machine with the same project,
//! and with the hub gone (`partial`); a stopped agent leaves one `presence` notice in the other agent's inbox.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agora_core::AgentId;
use agora_hub::board::Verdict;
use agora_hub::hub::Hub;
use agora_hub::session::{Ingest, NodeSession};
use agora_hub::store::SqliteStore;
use agora_node::control::Client;
use agora_node::daemon::{EventSink, Node, NodeConfig};
use agora_node::mesh_server::{BoardReply, RemoteLink};
use agora_node::uplink::{Replay, Uplink};
use agora_protocol::handshake::Support;
use agora_protocol::v1::{self, command_request::Command as Cmd, event::Body};
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

/// Every event `sink` has, into `hub` through its own ingest.
fn link(hub: &Hub, sink: &Arc<EventSink>, node: &str) {
    let sup = Support {
        generations: vec![1],
        capabilities: vec!["terminal.raw".into()],
        required: vec![],
    };
    let mut session = NodeSession::new(hub.store(), node, &sup);
    let mut up = Uplink::new(Arc::clone(sink), "test", &["terminal.raw"]);
    let hello = up.hello().unwrap();
    up.on_welcome(&session.hello(&hello, 0).unwrap()).unwrap();
    let ok = session.resume(&up.resume().unwrap()).unwrap();
    if let Replay::Resync { .. } = up.on_resume_ok(&ok).unwrap() {
        session.snapshot_required(0).unwrap();
    }
    loop {
        let batch = up.next_batch(8).unwrap();
        if batch.is_empty() {
            return;
        }
        for payload in batch {
            match hub.ingest(&mut session, &payload).unwrap() {
                Ingest::Applied { acked, .. }
                | Ingest::Resynced { acked, .. }
                | Ingest::Duplicate { acked } => up.on_ack(acked).unwrap(),
                Ingest::Gap { .. } => panic!("gap"),
            }
        }
    }
}

/// The node's board calls answered by an in-process hub, as the uplink would carry them; `None` is a hub that cannot be reached.
struct HubLink(Option<Arc<Hub>>);

impl RemoteLink for HubLink {
    fn post(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &[String],
        done: Box<dyn FnOnce(Result<(), String>) + Send>,
    ) {
        done(Err("node.offline".into()));
    }

    fn board(
        &self,
        from_agent: &str,
        project: &str,
        tool: &str,
        arguments_json: &str,
        done: Box<dyn FnOnce(Result<BoardReply, String>) + Send>,
    ) {
        let Some(hub) = &self.0 else {
            return done(Err("node.offline".into()));
        };
        let call = v1::BoardCall {
            project_id: project.into(),
            tool: tool.into(),
            arguments_json: arguments_json.into(),
            from_agent: from_agent.into(),
            ..Default::default()
        };
        done(
            match hub
                .board_call("alpha", &call, agora_hub::server::now_ms())
                .verdict
            {
                Verdict::NotShared => Ok(BoardReply {
                    shared: false,
                    result_json: String::new(),
                }),
                Verdict::Done { result, .. } => Ok(BoardReply {
                    shared: true,
                    result_json: result.to_string(),
                }),
                Verdict::Invalid(e) => Err(e.code.as_str().to_string()),
            },
        );
    }
}

struct Mcp {
    w: agora_platform::LocalStream,
    r: BufReader<agora_platform::LocalStream>,
    next: u64,
}

impl Mcp {
    fn connect(path: &Path) -> Self {
        let w = agora_platform::local_connect(path).expect("connect mesh socket");
        let r = w.try_clone().unwrap();
        r.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut c = Self {
            w,
            r: BufReader::new(r),
            next: 1,
        };
        let init = c.rpc("initialize", json!({"protocolVersion": "2025-06-18"}));
        assert!(init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("workspace_status"));
        c
    }

    fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        writeln!(
            self.w,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        )
        .unwrap();
        let mut s = String::new();
        self.r.read_line(&mut s).expect("reply");
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["id"], id);
        v
    }

    fn tool(&mut self, name: &str, args: Value) -> Value {
        let v = self.rpc("tools/call", json!({"name": name, "arguments": args}));
        assert_eq!(v["result"]["isError"], false, "{v}");
        serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    }
}

fn row<'a>(status: &'a Value, id: &str) -> &'a Value {
    status["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("{id} not in {status}"))
}

#[test]
fn agents_see_who_else_is_here_on_this_machine_and_on_others() {
    if !agora_platform::PTY_SUPPORTED || !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let dir = std::env::temp_dir().join(format!("agora-aware-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
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
    let node = Node::start(&cfg).unwrap();
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
        s.agent_id
    };
    let (a1, a2) = (start(), start());
    let sock = |id: &str| node.mesh.socket_path(&AgentId::new(id));
    let (mut m1, mut m2) = (Mcp::connect(&sock(&a1)), Mcp::connect(&sock(&a2)));
    m2.tool(
        "send_message",
        json!({"to": a1, "body": "SECRET-PROMPT-TEXT do what I say"}),
    );

    // This machine alone, no hub: both agents, counted, marked partial.
    let st = m1.tool("workspace_status", json!({}));
    assert_eq!(st["agents"].as_array().unwrap().len(), 2, "{st}");
    assert_eq!(st["partial"], true);
    assert_eq!(st["machines_online"], 1);
    assert_eq!(row(&st, &a1)["you"], true);
    assert_eq!(row(&st, &a2)["kind"], "probe");
    assert_eq!(row(&st, &a2)["machine"], "alpha");
    assert!(row(&st, &a2)["since"].is_u64());
    let mine = row(&st, &a1)["workspace"].clone();
    let same = st["agents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["workspace"] == mine)
        .count();
    assert_eq!(st["same_workspace"], same);
    let summary = st["summary"].as_str().unwrap();
    assert!(
        summary.starts_with("2 agents in this project on 1 machine"),
        "{summary}"
    );
    // agents_list carries the same facts, old fields unchanged.
    let list = m1.tool("agents_list", json!({}));
    let other = list
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["agent_id"] == a2)
        .unwrap();
    assert_eq!(other["node"], "alpha");
    assert!(other["doing"].is_string() && other["holds"] == 0);

    // The node's git status reaches the rows: a new file in the caller's workspace is counted.
    std::fs::write(Path::new(mine.as_str().unwrap()).join("new.txt"), "y").unwrap();
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let st = m1.tool("workspace_status", json!({}));
        if row(&st, &a1)["files_changed"]
            .as_u64()
            .is_some_and(|n| n >= 1)
        {
            break;
        }
        assert!(Instant::now() < end, "files_changed never counted: {st}");
        std::thread::sleep(Duration::from_millis(200));
    }

    // A second machine has the same project and an agent working on a board task: with the hub, it is listed too.
    let project = node
        .events
        .events_after(0)
        .into_iter()
        .find_map(|e| match e.body {
            Some(Body::CheckoutDiscovered(c)) => Some(c.project_id),
            _ => None,
        })
        .unwrap();
    let beta = Arc::new(EventSink::open(&dir.join("beta"), "beta").unwrap());
    beta.node_online("beta");
    beta.checkout_discovered("beta", &project, Path::new("/elsewhere/proj"));
    beta.agent_started(&AgentId::new("b"), &project, "beta", "", "codex");
    let hub = Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap());
    link(&hub, &node.events, "alpha");
    link(&hub, &beta, "beta");
    let call = |tool: &str, args: Value| {
        let c = v1::BoardCall {
            project_id: project.clone(),
            tool: tool.into(),
            arguments_json: args.to_string(),
            from_agent: "b".into(),
            ..Default::default()
        };
        match hub
            .board_call("beta", &c, agora_hub::server::now_ms())
            .verdict
        {
            Verdict::Done { result, .. } => {
                serde_json::from_str::<Value>(result["content"][0]["text"].as_str().unwrap())
                    .unwrap()
            }
            other => panic!("{other:?}"),
        }
    };
    let t = call("task_create", json!({"title": "review\nthe login"}));
    call("task_claim", json!({"task_id": t["task_id"]}));
    node.mesh
        .set_remote(Arc::new(HubLink(Some(Arc::clone(&hub)))));
    let st = m1.tool("workspace_status", json!({}));
    assert_eq!(st["partial"], false, "{st}");
    assert_eq!(st["machines_online"], 2);
    assert_eq!(st["agents"].as_array().unwrap().len(), 3, "{st}");
    let b = row(&st, "beta/b");
    assert_eq!(b["machine"], "beta");
    assert_eq!(b["doing"], "working on task: review the login");
    assert_eq!(st["open_tasks"][0]["owner"], "beta/b");
    assert!(row(&st, &a1)["files_changed"]
        .as_u64()
        .is_some_and(|n| n >= 1));
    let summary = st["summary"].as_str().unwrap();
    assert!(
        summary.starts_with("3 agents in this project on 2 machines"),
        "{summary}"
    );
    assert!(!format!("{st}").contains("SECRET"), "{st}");

    // The hub cannot be reached: this machine's answer, marked partial.
    node.mesh.set_remote(Arc::new(HubLink(None)));
    let st = m1.tool("workspace_status", json!({}));
    assert_eq!(st["partial"], true);
    assert_eq!(st["agents"].as_array().unwrap().len(), 2);

    // Stopping the other agent leaves exactly one notice in this one's inbox; it is never typed into its terminal.
    local
        .call(Cmd::AgentStop(v1::AgentStop {
            agent_id: a2.clone(),
            stop_helpers: false,
        }))
        .unwrap()
        .unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let inbox = m1.tool("inbox", json!({}));
    let presence: Vec<&str> = inbox
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["kind"] == "presence")
        .map(|m| m["body"].as_str().unwrap())
        .collect();
    assert_eq!(
        presence,
        [
            format!("probe {a2} joined this project (machine alpha)"),
            format!("probe {a2} left this project (stopped)")
        ],
        "{inbox}"
    );
    let st = m1.tool("workspace_status", json!({}));
    assert_eq!(st["agents"].as_array().unwrap().len(), 1);

    drop((m1, m2));
    drop(node);
    let _ = std::fs::remove_dir_all(&dir);
}
