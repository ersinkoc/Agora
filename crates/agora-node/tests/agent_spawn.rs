//! ADR-0057: an agent starts and follows helpers through its mesh tools, only with the user's signed decision. A real node and fake
//! processes; no accounts, tokens or real coding agents.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use agora_core::AgentId;
use agora_node::approvals::{start_signed_message, Pending, START_ACTION};
use agora_node::control::Client;
use agora_node::daemon::{Node, NodeConfig};
use agora_node::supervisor::NodeLimits;
use agora_platform::LocalStream;
use agora_protocol::v1::event::Body;
use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};
use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
use serde_json::{json, Value};

const NODE: &str = "node-spawn";
const DEVICE: &str = "dev-1";

fn fake() -> String {
    let p = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    static BUILD: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    BUILD.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let mut command = Command::new(cargo);
        command
            .args(["build", "-q", "-p", "agora-fake-agent", "--target-dir"])
            .arg(p.parent().unwrap().parent().unwrap());
        if p.parent().unwrap().ends_with("release") {
            command.arg("--release");
        }
        assert!(
            command.status().unwrap().success(),
            "building fake fixture failed"
        );
    });
    p.to_string_lossy().into_owned()
}

struct Env {
    root: PathBuf,
    folder: PathBuf,
    key: EcdsaKeyPair,
    node: Option<Node>,
}

impl Env {
    fn new(name: &str) -> Self {
        Self::with_secret(name, None)
    }

    /// [`Self::new`] on a node that holds `secret` in its store (#1147).
    fn with_secret(name: &str, secret: Option<&str>) -> Self {
        let root = std::env::temp_dir().join(format!("spawn-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("manifests")).unwrap();
        std::fs::create_dir_all(root.join("data")).unwrap();
        if let Some(v) = secret {
            agora_node::credentials::SecretStore::in_data(&root.join("data"))
                .set("TOKEN", &agora_node::credentials::Secret::new(v).unwrap())
                .unwrap();
        }
        let folder = root.join("work");
        std::fs::create_dir_all(folder.join("sub")).unwrap();
        // `probe` stays up reading its input; `screen` shows a line and holds it.
        let launch = serde_json::to_string(&vec![
            fake(),
            "--probe-directory".into(),
            root.join("probe-{pid}.json").to_string_lossy().into_owned(),
        ])
        .unwrap();
        std::fs::write(
            root.join("manifests/probe.toml"),
            // An agent (it has a mesh section), so a grant may cover it; `screen` is a plain terminal program, which no grant covers.
            // ADR-0059: it declares an option that skips permissions; a helper must never get it.
            format!("kind = \"probe\"\ndisplay_name = \"Probe\"\nversion = \"1\"\nmin_engine = 3\n[process]\nlaunch = {launch}\n[[launch.option]]\nid = \"skip-permissions\"\nlabel = \"option-skip-permissions\"\nargs = [\"--fake-skip-permissions\"]\nskips_permissions = true\n[[launch.setting]]\nid = \"model\"\nlabel = \"setting-model\"\nargs = [\"--fake-model\", \"{{value}}\"]\n[[launch.setting]]\nid = \"effort\"\nlabel = \"setting-effort\"\nargs = [\"--fake-effort\", \"{{value}}\"]\nvalues = [\"low\", \"high\"]\n[mcp]\nmethod = \"cli_flag\"\npath = \".mcp.json\"\nflag = [\"--mcp-config\", \"{{config_file}}\"]\n[[typed]]\nid = \"t\"\nstate = \"idle\"\npriority = 1\nregion = \"bottom_non_empty_lines(12)\"\nregex = \"^> \"\nall = [{{ contains = \"PROBE-FOOTER\" }}]\n"),
        )
        .unwrap();
        std::fs::write(
            root.join("screen.toml"),
            "[[frame]]\ntext = \"HELLO-HELPER\\n> \"\ndelay_ms = 120000\n",
        )
        .unwrap();
        let screen = serde_json::to_string(&vec![
            fake(),
            "--script".into(),
            root.join("screen.toml").to_string_lossy().into_owned(),
        ])
        .unwrap();
        std::fs::write(
            root.join("manifests/screen.toml"),
            format!("kind = \"screen\"\ndisplay_name = \"Screen\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = {screen}\n"),
        )
        .unwrap();
        // A structured agent: the fake L3 adapter (ACP agents go through the same adapter path, `agora acp-bridge`).
        std::fs::write(
            root.join("manifests/adapter.toml"),
            format!("kind = \"adapter\"\ndisplay_name = \"Adapter\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\"]\n", fake()),
        )
        .unwrap();
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
            .unwrap();
        let hex: String = key
            .public_key()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        std::fs::write(
            root.join("data/trusted-devices"),
            format!("{DEVICE} {hex}\n"),
        )
        .unwrap();
        let node = Node::start(&NodeConfig {
            resume_all: false,
            node_id: NODE.into(),
            socket: root.join("n.sock"),
            data_dir: root.join("data"),
            worktree_root: root.join("worktrees"),
            manifests_dir: root.join("manifests"),
            projects: vec![],
            limits: NodeLimits::default(),
        })
        .unwrap();
        Self {
            root,
            folder,
            key,
            node: Some(node),
        }
    }

    fn node(&self) -> &Node {
        self.node.as_ref().unwrap()
    }

    fn client(&self) -> Client {
        Client::connect(self.node().socket().unwrap()).unwrap()
    }

    /// A user-started agent in `folder`.
    fn launch(&self, folder: &Path, kind: &str) -> String {
        self.launch_with(folder, kind, &[])
    }

    /// [`Self::launch`] with launch options (ADR-0059).
    fn launch_with(&self, folder: &Path, kind: &str, options: &[&str]) -> String {
        let mut c = self.client();
        let info = match c
            .call(Cmd::DirectoryInspect(v1::DirectoryInspect {
                path: folder.to_string_lossy().into_owned(),
                create_project_id: false,
                authorization: None,
            }))
            .unwrap()
            .unwrap()
            .result
        {
            Some(Ok_::DirectoryInfo(i)) => i,
            other => panic!("{other:?}"),
        };
        match c
            .call(Cmd::AgentLaunch(v1::AgentLaunch {
                path: info.path.clone(),
                project_id: info.project_id.clone(),
                kind: kind.into(),
                launch_options: options
                    .iter()
                    .map(|o| o.trim_end_matches('!').to_string())
                    .collect(),
                // `id!` in these tests: the user confirmed it (a separate signed list, never part of the id).
                confirmed_launch_options: options
                    .iter()
                    .filter_map(|o| o.strip_suffix('!').map(str::to_string))
                    .collect(),
                ..Default::default()
            }))
            .unwrap()
            .unwrap()
            .result
        {
            Some(Ok_::AgentLaunch(r)) => r.agent_id,
            other => panic!("{other:?}"),
        }
    }

    fn mcp(&self, agent: &str) -> Mcp {
        Mcp::connect(&self.node().mesh.socket_path(&AgentId::new(agent)))
    }

    /// The `agent.start` request `agent` is waiting on. It is awaited for up to 30 s. With 10 s, a busy Windows machine running this
    /// file's tests in parallel (each with its own node and agents) missed the request: 13 of 14 tests failed here on main, and 2
    /// of 11 at v0.3.37. Each test passed alone (about 22-28 s) and the whole file passed single-threaded (#1150, 2026-11-25).
    fn request_of(&self, agent: &str) -> Pending {
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(p) = self
                .node()
                .approvals
                .waiting()
                .into_iter()
                .find(|p| p.agent_id == agent && p.action == START_ACTION)
            {
                return p;
            }
            assert!(Instant::now() < end, "no start request from {agent}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn sign(&self, message: &[u8]) -> Vec<u8> {
        self.key
            .sign(&SystemRandom::new(), message)
            .unwrap()
            .as_ref()
            .to_vec()
    }

    /// The user's decision, signed over the request as it was announced.
    fn decide(
        &self,
        p: &Pending,
        approved: bool,
        grant: u32,
        nested: bool,
    ) -> Result<v1::CommandOk, v1::ErrorInfo> {
        let sig = self.sign(&start_signed_message(
            NODE, &p.id, approved, grant, nested, &p.detail,
        ));
        self.resolve(p, approved, grant, nested, sig)
    }

    fn resolve(
        &self,
        p: &Pending,
        approved: bool,
        grant: u32,
        nested: bool,
        signature: Vec<u8>,
    ) -> Result<v1::CommandOk, v1::ErrorInfo> {
        self.client()
            .call(Cmd::ApprovalResolve(v1::ApprovalResolve {
                approval_id: p.id.clone(),
                approved,
                signer_device_id: DEVICE.into(),
                signature,
                helper_grant: grant,
                helpers_may_start: nested,
                ..Default::default()
            }))
            .unwrap()
    }

    fn running(&self, agent: &str) -> bool {
        self.node()
            .supervisor
            .lock()
            .unwrap()
            .list()
            .iter()
            .any(|v| v.id.as_str() == agent && v.running)
    }

    fn stop(&self, agent: &str, helpers: bool) {
        self.client()
            .call(Cmd::AgentStop(v1::AgentStop {
                agent_id: agent.into(),
                stop_helpers: helpers,
            }))
            .unwrap()
            .unwrap();
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        self.node.take();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct Mcp {
    writer: LocalStream,
    reader: BufReader<LocalStream>,
    next: u64,
}

impl Mcp {
    fn connect(path: &Path) -> Self {
        let writer = agora_platform::local_connect(path).unwrap();
        let reader = BufReader::new(writer.try_clone().unwrap());
        reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        let mut m = Self {
            writer,
            reader,
            next: 0,
        };
        let id = m.send("initialize", json!({}));
        assert_eq!(m.reply()["id"], id);
        m
    }

    fn send(&mut self, method: &str, params: Value) -> u64 {
        self.next += 1;
        writeln!(
            self.writer,
            "{}",
            json!({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params})
        )
        .unwrap();
        self.next
    }

    fn call(&mut self, tool: &str, args: Value) -> u64 {
        self.send("tools/call", json!({"name": tool, "arguments": args}))
    }

    fn reply(&mut self) -> Value {
        let mut line = String::new();
        self.reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    /// The parsed text of a tool result and whether it is an error.
    fn result(&mut self, id: u64) -> (Value, bool) {
        let r = self.reply();
        assert_eq!(r["id"], id, "{r}");
        let text = r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        (
            serde_json::from_str(&text).unwrap(),
            r["result"]["isError"] == true,
        )
    }

    fn error_code(&mut self, id: u64) -> String {
        let (v, is_error) = self.result(id);
        assert!(is_error, "{v}");
        v["error"]["code"].as_str().unwrap().to_string()
    }
}

fn supported() -> bool {
    agora_platform::PTY_SUPPORTED && agora_platform::LOCAL_SOCKETS_SUPPORTED
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < end, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_start_waits_for_the_signed_decision_and_shows_the_whole_request() {
    if !supported() {
        return;
    }
    let e = Env::new("approve");
    let parent = e.launch(&e.folder, "probe");
    let mut m = e.mcp(&parent);
    let list = m.send("tools/list", json!({}));
    let tools = m.reply();
    assert_eq!(tools["id"], list);
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for t in ["agent_start", "agent_wait", "agent_output"] {
        assert!(names.contains(&t), "{t} is not listed");
    }
    let prompt = "Write the tests for src/parser.rs.\nKeep them small.";
    let call = m.call("agent_start", json!({"kind": "screen", "prompt": prompt}));
    let p = e.request_of(&parent);
    let detail: Value = serde_json::from_str(&p.detail).unwrap();
    assert_eq!(detail["kind"], "screen");
    assert_eq!(detail["prompt"], prompt, "the prompt is shown in full");
    assert_eq!(detail["agent"], parent.as_str());
    assert_eq!(detail["node"], NODE);
    assert_eq!(detail["inside"], true);
    assert_eq!(
        Path::new(detail["folder"].as_str().unwrap()),
        agora_platform::canonicalize_plain(&e.folder).unwrap()
    );
    assert!(
        e.node().supervisor.lock().unwrap().list().len() == 1,
        "nothing starts before the decision"
    );

    // The ADR-0028 message does not decide a start, nor does a decision over a different request (a hub that swapped the prompt).
    let plain = e.sign(&agora_node::approvals::signed_message(NODE, &p.id, true));
    assert_eq!(
        e.resolve(&p, true, 0, false, plain).unwrap_err().code,
        "approval.signature_invalid"
    );
    let swapped = e.sign(&start_signed_message(
        NODE,
        &p.id,
        true,
        0,
        false,
        &p.detail.replace("parser", "secrets"),
    ));
    assert_eq!(
        e.resolve(&p, true, 0, false, swapped).unwrap_err().code,
        "approval.signature_invalid"
    );
    // A grant the user did not sign is not accepted either.
    let without_grant = e.sign(&start_signed_message(
        NODE, &p.id, true, 0, false, &p.detail,
    ));
    assert_eq!(
        e.resolve(&p, true, 3, false, without_grant)
            .unwrap_err()
            .code,
        "approval.signature_invalid"
    );

    e.decide(&p, true, 0, false).unwrap();
    let (v, is_error) = m.result(call);
    assert!(!is_error, "{v}");
    let helper = v["agent_id"].as_str().unwrap().to_string();
    assert!(e.running(&helper));
    // The tree: the helper's AgentStarted names its parent, and the decision is recorded.
    let events = e.node().events.events_after(0);
    assert!(events.iter().any(|ev| matches!(&ev.body, Some(Body::AgentStarted(s)) if s.agent_id == helper && s.parent_agent_id == parent)));
    assert!(events.iter().any(|ev| matches!(&ev.body, Some(Body::ApprovalResolved(r)) if r.approval_id == p.id && r.approved && r.signer_device_id == DEVICE)));

    // Output: the bottom of its screen, then its first prompt typed into it.
    wait_until("helper screen", || {
        let id = m.call("agent_output", json!({"agent_id": helper}));
        let (v, _) = m.result(id);
        v["lines"].as_array().is_some_and(|l| {
            l.iter()
                .any(|x| x.as_str().is_some_and(|s| s.contains("HELLO-HELPER")))
        })
    });

    // Waiting: a state it does not reach times out; once stopped, the wait ends with `exited`.
    let w = m.call(
        "agent_wait",
        json!({"agent_id": helper, "state": "blocked", "timeout_s": 0.3}),
    );
    assert_eq!(m.error_code(w), "mesh.timeout");
    let w = m.call(
        "agent_wait",
        json!({"agent_id": helper, "state": "done", "timeout_s": 20}),
    );
    e.stop(&helper, false);
    let (v, is_error) = m.result(w);
    assert!(!is_error && v["state"] == "exited", "{v}");
}

#[test]
fn a_refusal_a_grant_and_the_limits() {
    if !supported() {
        return;
    }
    let e = Env::new("grant");
    let parent = e.launch(&e.folder, "probe");
    let mut m = e.mcp(&parent);

    // Refused by the user.
    let call = m.call("agent_start", json!({"kind": "probe", "prompt": "a"}));
    let p = e.request_of(&parent);
    // One request waits for the user at a time.
    let second = m.call("agent_start", json!({"kind": "probe"}));
    assert_eq!(m.error_code(second), "mesh.helper_limit");
    e.decide(&p, false, 0, false).unwrap();
    assert_eq!(m.error_code(call), "mesh.start_denied");

    // Approved with a grant for three: the next two of the same kind in the same folder start without asking.
    let call = m.call("agent_start", json!({"kind": "probe", "prompt": "b"}));
    let p = e.request_of(&parent);
    e.decide(&p, true, 3, false).unwrap();
    let (v, _) = m.result(call);
    let first = v["agent_id"].as_str().unwrap().to_string();
    let grants = e.node().grants.list(0);
    let g = grants
        .iter()
        .find(|g| g.grant_id == p.id)
        .expect("the grant is listed");
    assert_eq!(
        (g.kind.as_str(), g.remaining, g.scope.clone()),
        ("probe", 2, format!("agent:{parent}"))
    );
    let call = m.call(
        "agent_start",
        json!({"kind": "probe", "folder": e.folder.join("sub").to_string_lossy()}),
    );
    // A subfolder is another folder for the grant: it asks.
    let q = e.request_of(&parent);
    assert_eq!(
        serde_json::from_str::<Value>(&q.detail).unwrap()["inside"],
        true
    );
    e.decide(&q, true, 0, false).unwrap();
    let (v, is_error) = m.result(call);
    assert!(!is_error, "{v}");
    let call = m.call("agent_start", json!({"kind": "probe"}));
    let (v, is_error) = m.result(call);
    assert!(!is_error, "the grant covers it: {v}");
    // Three helpers run: the fourth is refused before anyone is asked, even under the grant, and that is recorded.
    let call = m.call("agent_start", json!({"kind": "probe"}));
    assert_eq!(m.error_code(call), "mesh.helper_limit");
    assert!(e.node().events.events_after(0).iter().any(|ev| matches!(&ev.body, Some(Body::AgentStartRefused(r)) if r.agent_id == parent && r.code == "mesh.helper_limit")));

    // A helper cannot start helpers by default.
    let mut h = e.mcp(&first);
    let call = h.call("agent_start", json!({"kind": "probe"}));
    assert_eq!(h.error_code(call), "mesh.helper_limit");
    // Another agent cannot follow or read someone else's helper.
    let stranger = e.launch(&e.folder, "probe");
    let mut s = e.mcp(&stranger);
    let call = s.call("agent_output", json!({"agent_id": first}));
    assert_eq!(s.error_code(call), "agent.not_found");
    let call = s.call("agent_wait", json!({"agent_id": first, "state": "exited"}));
    assert_eq!(s.error_code(call), "agent.not_found");

    // Rate: six attempts a minute, refused ones included; the seventh is refused as too fast.
    let _asking = s.call("agent_start", json!({"kind": "probe"}));
    e.request_of(&stranger);
    for _ in 0..5 {
        let c = s.call("agent_start", json!({"kind": "probe"}));
        assert_eq!(s.error_code(c), "mesh.helper_limit");
    }
    let c = s.call("agent_start", json!({"kind": "probe"}));
    assert_eq!(s.error_code(c), "mesh.rate_limited");
    let c = s.call("agent_start", json!({"kind": "probe"}));
    assert_eq!(s.error_code(c), "mesh.rate_limited");
    // A looping agent does not fill the log: one rate refusal on record per minute.
    let rate_records = e
        .node()
        .events
        .events_after(0)
        .iter()
        .filter(|ev| matches!(&ev.body, Some(Body::AgentStartRefused(r)) if r.agent_id == stranger && r.code == "mesh.rate_limited"))
        .count();
    assert_eq!(rate_records, 1);

    // Stopping the parent with its helpers stops them, and its grant ends with it.
    let helpers: Vec<String> = e
        .node()
        .supervisor
        .lock()
        .unwrap()
        .list()
        .into_iter()
        .filter(|v| v.parent.as_ref().is_some_and(|p| p.as_str() == parent) && v.running)
        .map(|v| v.id.as_str().to_string())
        .collect();
    assert_eq!(helpers.len(), 3);
    e.stop(&parent, true);
    for h in &helpers {
        assert!(!e.running(h), "{h} still runs");
    }
    assert!(e.node().approvals.helper_grants().is_empty());
}

/// #1119 item 3: a node that is stopping starts no helper, even one a grant covers (it would launch under the lock the stop takes to
/// let its agents go, and nothing counts it): the agent is told `node.offline`, as a command is.
#[test]
fn a_node_that_is_stopping_starts_no_helper_even_under_a_grant() {
    if !supported() {
        return;
    }
    let e = Env::new("stopping");
    let parent = e.launch(&e.folder, "probe");
    let mut m = e.mcp(&parent);
    let call = m.call("agent_start", json!({"kind": "probe"}));
    let p = e.request_of(&parent);
    e.decide(&p, true, 3, false).unwrap();
    let (v, is_error) = m.result(call);
    assert!(!is_error, "{v}");
    e.node().begin_stop();
    let call = m.call("agent_start", json!({"kind": "probe"}));
    assert_eq!(m.error_code(call), "node.offline");
    assert_eq!(
        helpers_of(&e, &parent),
        1,
        "a helper started while the node stopped"
    );
    assert_eq!(
        refusals(&e, &parent, "node.offline"),
        1,
        "the refusal is not on record"
    );
}

/// How many helpers `parent` has (running or not).
fn helpers_of(e: &Env, parent: &str) -> usize {
    e.node()
        .supervisor
        .lock()
        .unwrap()
        .list()
        .into_iter()
        .filter(|v| v.parent.as_ref().is_some_and(|p| p.as_str() == parent))
        .count()
}

/// How many `agent_start_refused` events with `code` the node recorded for `parent`.
fn refusals(e: &Env, parent: &str, code: &str) -> usize {
    e.node()
        .events
        .events_after(0)
        .iter()
        .filter(|ev| matches!(&ev.body, Some(Body::AgentStartRefused(r)) if r.agent_id == parent && r.code == code))
        .count()
}

/// #1119 item 3 (review of #1132): a decision that was already being handled when the node began to stop (its command was admitted
/// before) reaches the launch, which reads the stop under the supervisor's lock: nothing starts, and the refusal is on record.
#[test]
fn a_decision_handled_as_the_node_stops_starts_no_helper() {
    if !supported() {
        return;
    }
    let e = Env::new("stopping-decided");
    let parent = e.launch(&e.folder, "probe");
    let mut m = e.mcp(&parent);
    let call = m.call("agent_start", json!({"kind": "probe"}));
    let p = e.request_of(&parent);
    e.node().begin_stop();
    // As the command that verified the decision does next (it was admitted before the stop; a new one is refused).
    e.node()
        .spawner
        .decided(
            &p,
            &v1::ApprovalResolve {
                approval_id: p.id.clone(),
                approved: true,
                signer_device_id: DEVICE.into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(m.error_code(call), "node.offline");
    assert_eq!(
        helpers_of(&e, &parent),
        0,
        "a helper started while the node stopped"
    );
    assert_eq!(
        refusals(&e, &parent, "node.offline"),
        1,
        "the refusal is not on record"
    );
}

#[test]
fn the_folder_ceiling_and_a_decision_that_lets_a_helper_ask() {
    if !supported() {
        return;
    }
    let e = Env::new("folder");
    let parent = e.launch(&e.folder, "probe");
    let mut m = e.mcp(&parent);
    // A prompt with control characters (hidden keys) is refused before anyone is asked.
    let c = m.call(
        "agent_start",
        json!({"kind": "probe", "prompt": "ok\u{1b}[2Jrm -rf ~\r"}),
    );
    assert_eq!(m.error_code(c), "protocol.invalid_message");
    // A folder that does not exist is refused.
    let call = m.call(
        "agent_start",
        json!({"kind": "probe", "folder": e.root.join("nowhere").to_string_lossy()}),
    );
    assert_eq!(m.error_code(call), "mesh.folder_not_allowed");
    // A folder outside the asker's is asked about, marked as outside, and a grant signed with it is not kept.
    let outside = e.root.join("manifests");
    let call = m.call(
        "agent_start",
        json!({"kind": "probe", "folder": outside.to_string_lossy()}),
    );
    let p = e.request_of(&parent);
    assert_eq!(
        serde_json::from_str::<Value>(&p.detail).unwrap()["inside"],
        false
    );
    e.decide(&p, true, 5, true).unwrap();
    let (v, _) = m.result(call);
    let out_helper = v["agent_id"].as_str().unwrap().to_string();
    assert!(e.node().approvals.helper_grants().is_empty());
    // ...and a helper started outside may not ask, whatever was signed.
    let mut o = e.mcp(&out_helper);
    let c = o.call("agent_start", json!({"kind": "probe"}));
    assert_eq!(o.error_code(c), "mesh.helper_limit");

    // Inside, a decision that allows it lets the helper ask (one level only).
    let call = m.call("agent_start", json!({"kind": "probe"}));
    let p = e.request_of(&parent);
    e.decide(&p, true, 0, true).unwrap();
    let (v, _) = m.result(call);
    let helper = v["agent_id"].as_str().unwrap().to_string();
    let mut h = e.mcp(&helper);
    // Its own helper must stay inside its folder.
    let c = h.call(
        "agent_start",
        json!({"kind": "probe", "folder": outside.to_string_lossy()}),
    );
    assert_eq!(h.error_code(c), "mesh.folder_not_allowed");
    let c = h.call("agent_start", json!({"kind": "probe"}));
    let p = e.request_of(&helper);
    assert_eq!(
        serde_json::from_str::<Value>(&p.detail).unwrap()["depth"],
        2
    );
    e.decide(&p, true, 0, true).unwrap();
    let (v, _) = h.result(c);
    let grandchild = v["agent_id"].as_str().unwrap().to_string();
    let mut g = e.mcp(&grandchild);
    let c = g.call("agent_start", json!({"kind": "probe"}));
    assert_eq!(g.error_code(c), "mesh.helper_limit");
    // An agent that goes away withdraws what it was waiting for.
    let _waiting = h.call("agent_start", json!({"kind": "screen"}));
    let p = e.request_of(&helper);
    e.stop(&helper, false);
    wait_until("withdrawn", || {
        !e.node().approvals.waiting().iter().any(|w| w.id == p.id)
    });
    assert!(
        e.decide(&p, true, 0, false).is_err(),
        "a decision on a withdrawn request starts nothing"
    );
}

#[test]
fn concurrent_starts_shells_recorded_prompts_and_failed_launches() {
    if !supported() {
        return;
    }
    let e = Env::new("race");
    let parent = e.launch(&e.folder, "probe");
    // Two connections of one agent ask at the same moment: one request reaches the user, the other is refused.
    let (mut a, mut b) = (e.mcp(&parent), e.mcp(&parent));
    let ca = a.call("agent_start", json!({"kind": "probe", "prompt": "one"}));
    let cb = b.call("agent_start", json!({"kind": "probe", "prompt": "two"}));
    let p = e.request_of(&parent);
    std::thread::sleep(Duration::from_millis(300));
    let waiting = e
        .node()
        .approvals
        .waiting()
        .into_iter()
        .filter(|w| w.agent_id == parent)
        .count();
    assert_eq!(waiting, 1);
    e.decide(&p, false, 0, false).unwrap();
    let mut codes = vec![a.error_code(ca), b.error_code(cb)];
    codes.sort();
    assert_eq!(codes, vec!["mesh.helper_limit", "mesh.start_denied"]);

    // A plain terminal program is never covered by a grant, and the request says so.
    let call = a.call("agent_start", json!({"kind": "screen"}));
    let p = e.request_of(&parent);
    assert_eq!(
        serde_json::from_str::<Value>(&p.detail).unwrap()["grant_allowed"],
        false
    );
    e.decide(&p, true, 3, true).unwrap();
    let (v, is_error) = a.result(call);
    assert!(!is_error, "{v}");
    assert!(e.node().approvals.helper_grants().is_empty());
    let resolved = e
        .node()
        .events
        .events_after(0)
        .into_iter()
        .find_map(|ev| match ev.body {
            Some(Body::ApprovalResolved(r)) if r.approval_id == p.id => Some(r),
            _ => None,
        });
    assert_eq!(
        resolved.map(|r| (r.helper_grant, r.helpers_may_start)),
        Some((0, false))
    );

    // Commands for the agent itself are refused; a relative folder is below the asker's own.
    for bad in ["!rm -rf ~", "/clear"] {
        let c = a.call("agent_start", json!({"kind": "probe", "prompt": bad}));
        assert_eq!(a.error_code(c), "protocol.invalid_message", "{bad:?}");
    }
    let call = b.call(
        "agent_start",
        json!({"kind": "probe", "folder": "sub", "prompt": "in sub"}),
    );
    let p = e.request_of(&parent);
    let detail: Value = serde_json::from_str(&p.detail).unwrap();
    assert_eq!(
        Path::new(detail["folder"].as_str().unwrap()),
        agora_platform::canonicalize_plain(&e.folder.join("sub")).unwrap()
    );
    assert_eq!(detail["inside"], true);
    // The folder went away before the user answered: nothing starts, no grant is given, and the failure is on record.
    std::fs::remove_dir_all(e.folder.join("sub")).unwrap();
    e.decide(&p, true, 3, false).unwrap();
    assert!(b.result(call).1);
    let events = e.node().events.events_after(0);
    assert!(events.iter().any(|ev| matches!(&ev.body, Some(Body::ApprovalResolved(r)) if r.approval_id == p.id && r.approved && r.helper_grant == 0)));
    assert!(events.iter().any(|ev| matches!(&ev.body, Some(Body::AgentStartRefused(r)) if r.agent_id == parent && r.folder.ends_with("sub"))));
    assert!(e.node().approvals.helper_grants().is_empty());
}

#[test]
fn granted_prompts_are_recorded_and_waits_are_capped() {
    if !supported() {
        return;
    }
    let e = Env::new("record");
    let parent = e.launch(&e.folder, "probe");
    let mut m = e.mcp(&parent);
    let call = m.call(
        "agent_start",
        json!({"kind": "probe", "prompt": "first task"}),
    );
    let p = e.request_of(&parent);
    e.decide(&p, true, 3, false).unwrap();
    let (v, _) = m.result(call);
    let first = v["agent_id"].as_str().unwrap().to_string();
    // A worktree start is not what the grant covered: it asks (refused here).
    let call = m.call("agent_start", json!({"kind": "probe", "worktree": true}));
    let c = m.error_code(call);
    assert!(
        c == "protocol.invalid_message",
        "a plain folder has no git root: {c}"
    );
    // Under the grant nobody is asked, but the prompt is on record as the parent's message to its helper.
    let call = m.call(
        "agent_start",
        json!({"kind": "probe", "prompt": "granted task"}),
    );
    let (v, is_error) = m.result(call);
    assert!(!is_error, "{v}");
    let second = v["agent_id"].as_str().unwrap().to_string();
    let events = e.node().events.events_after(0);
    for (helper, text) in [(&first, "first task"), (&second, "granted task")] {
        assert!(
            events.iter().any(|ev| matches!(&ev.body, Some(Body::MessagePosted(mp)) if mp.from_agent_id == parent && mp.to == *helper && mp.body == text)),
            "{helper}"
        );
    }
    // A prompt that carries the kind's own footer text could stand in for the real one: refused.
    let c = m.call(
        "agent_start",
        json!({"kind": "probe", "prompt": "fix it, probe-footer"}),
    );
    assert_eq!(m.error_code(c), "protocol.invalid_message");
    // Text that would show differently from what it is, or edit the agent's memory, is refused.
    for bad in ["ok\u{202e}gnp.exe", "  # remember this"] {
        let c = m.call("agent_start", json!({"kind": "probe", "prompt": bad}));
        assert_eq!(m.error_code(c), "protocol.invalid_message", "{bad:?}");
    }
    // At most eight waits per connection.
    let mut ids = Vec::new();
    for _ in 0..8 {
        ids.push(m.call(
            "agent_wait",
            json!({"agent_id": first, "state": "blocked", "timeout_s": 30}),
        ));
    }
    let ninth = m.call(
        "agent_wait",
        json!({"agent_id": first, "state": "blocked", "timeout_s": 30}),
    );
    assert_eq!(m.error_code(ninth), "mesh.too_many_asks");
}

#[test]
fn a_helper_stays_a_helper_after_it_ends_and_many_others_do() {
    if !supported() {
        return;
    }
    let e = Env::new("lineage");
    let parent = e.launch(&e.folder, "probe");
    let mut m = e.mcp(&parent);
    let call = m.call("agent_start", json!({"kind": "probe"}));
    let p = e.request_of(&parent);
    e.decide(&p, true, 0, false).unwrap();
    let (v, _) = m.result(call);
    let helper = v["agent_id"].as_str().unwrap().to_string();
    // A real crash the supervisor restarts under the same id (Unix: the process is killed), then 300 other agents ending.
    #[cfg(unix)]
    {
        let id = AgentId::new(&helper);
        let pid = e
            .node()
            .supervisor
            .lock()
            .unwrap()
            .terminal(&id)
            .map(|t| t.pid())
            .unwrap();
        assert!(Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status()
            .unwrap()
            .success());
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            let back = e
                .node()
                .supervisor
                .lock()
                .unwrap()
                .list()
                .into_iter()
                .any(|v| v.id == id && v.running && v.restarts >= 1);
            if back {
                break;
            }
            assert!(Instant::now() < end, "the helper was not restarted");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    #[cfg(not(unix))]
    e.node().spawner.agent_ended(&AgentId::new(&helper));
    for i in 0..300 {
        e.node()
            .spawner
            .agent_ended(&AgentId::new(format!("other-{i}")));
    }
    // Still a helper at depth 1 that its decision did not allow to ask: no start, not even in an outside folder.
    let mut h = e.mcp(&helper);
    let c = h.call("agent_start", json!({"kind": "probe"}));
    assert_eq!(h.error_code(c), "mesh.helper_limit");
    let c = h.call(
        "agent_start",
        json!({"kind": "probe", "folder": e.root.join("manifests").to_string_lossy()}),
    );
    assert_eq!(h.error_code(c), "mesh.helper_limit");
    // And the parent still counts it, and still follows it.
    let w = m.call(
        "agent_wait",
        json!({"agent_id": helper, "state": "exited", "timeout_s": 0.2}),
    );
    assert_eq!(m.error_code(w), "mesh.timeout");
}

/// The agent's transcript text, once an agent entry satisfies `done`.
fn agent_says(e: &Env, agent: &str, prompt: &str, done: impl Fn(&str) -> bool) -> String {
    use agora_node::control::{KIND_CHAT_ENTRY, KIND_CHAT_SNAPSHOT};
    use prost::Message as _;
    let mut c = e.client();
    c.call_on(
        3,
        Cmd::ChatSubscribe(v1::ChatSubscribe {
            agent_id: agent.into(),
        }),
    )
    .unwrap()
    .unwrap();
    c.call(Cmd::AgentPrompt(v1::AgentPrompt {
        agent_id: agent.into(),
        text: prompt.into(),
    }))
    .unwrap()
    .unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        let Some(f) = c.read_frame(Some(Duration::from_millis(200))).unwrap() else {
            continue;
        };
        let entries = match (f.stream_id, f.kind) {
            (3, KIND_CHAT_SNAPSHOT) => {
                v1::ChatSnapshot::decode(f.payload.as_slice())
                    .unwrap()
                    .entries
            }
            (3, KIND_CHAT_ENTRY) => vec![v1::ChatEntry::decode(f.payload.as_slice()).unwrap()],
            _ => continue,
        };
        if let Some(t) = entries
            .into_iter()
            .find(|en| en.role == v1::ChatRole::Agent as i32 && done(&en.text))
        {
            return t.text;
        }
    }
    panic!("{agent} did not answer {prompt}");
}

/// Structured agents (L3 adapters and L2 ACP agents through the bridge) get the mesh too: the node registers them, passes their own
/// endpoint in `session/start` (`mcp_servers`), other agents see them in `agents_list`, and their `agent_start` waits for the user
/// like any agent's.
#[test]
fn an_adapter_agent_is_in_the_mesh_and_its_start_waits_for_the_user() {
    if !supported() {
        return;
    }
    let e = Env::new("adapter-mesh");
    let structured = e.launch(&e.folder, "adapter");
    let own = e.node().mesh.socket_path(&AgentId::new(&structured));
    let said = agent_says(&e, &structured, "mcp-servers", |t| t.starts_with('['));
    let servers: Value = serde_json::from_str(&said).unwrap();
    assert_eq!(servers.as_array().unwrap().len(), 1, "{servers}");
    assert_eq!(servers[0]["name"], "agora-mesh");
    assert_eq!(
        servers[0]["command"],
        std::env::current_exe().unwrap().to_string_lossy().as_ref(),
        "the node's own program, as for a terminal agent"
    );
    assert_eq!(
        servers[0]["args"],
        json!(["mesh", "stdio", "--socket", own.to_string_lossy()])
    );

    // Another agent sees it in the mesh.
    let other = e.launch(&e.folder, "probe");
    let mut m = e.mcp(&other);
    let id = m.call("agents_list", json!({}));
    let (list, is_error) = m.result(id);
    assert!(!is_error, "{list}");
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .any(|a| a["agent_id"] == structured.as_str() && a["kind"] == "adapter"),
        "{list}"
    );

    // Its own endpoint is its identity in the mesh: a start it asks for waits for the user's decision.
    let mut mine = Mcp::connect(&own);
    let _call = mine.call("agent_start", json!({"kind": "screen", "prompt": "hello"}));
    let p = e.request_of(&structured);
    assert_eq!(p.action, START_ACTION);
    let detail: Value = serde_json::from_str(&p.detail).unwrap();
    assert_eq!(detail["agent"], structured.as_str());
    assert_eq!(detail["kind"], "screen");

    // A stop takes it out of the mesh and closes its endpoint; a resume joins again and passes the new endpoint.
    e.stop(&structured, false);
    wait_until("endpoint closed", || {
        !e.node().mesh.is_registered(&AgentId::new(&structured))
    });
    e.node()
        .supervisor
        .lock()
        .unwrap()
        .resume(&AgentId::new(&structured), Instant::now())
        .unwrap();
    assert!(e.node().mesh.is_registered(&AgentId::new(&structured)));
    let said = agent_says(&e, &structured, "mcp-servers", |t| t.starts_with('['));
    let servers: Value = serde_json::from_str(&said).unwrap();
    assert_eq!(servers[0]["args"][3], own.to_string_lossy().as_ref());
}

/// The command lines the probe agents reported, by the folder each ran in.
fn probe_args(root: &Path) -> Vec<Vec<String>> {
    std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("probe-"))
        .filter_map(|e| serde_json::from_slice::<Value>(&std::fs::read(e.path()).ok()?).ok())
        .map(|v| {
            v["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a.as_str().unwrap().to_string())
                .collect()
        })
        .collect()
}

/// ADR-0059: an agent started with an option that skips permissions starts helpers without it. The helper is started from its plain
/// manifest command (no option, no badge) even though the parent has one, its `AgentStarted` names none, and a grant (the decision's
/// "up to N like this") does not change that.
#[test]
fn a_helper_never_inherits_a_launch_option() {
    if !supported() {
        return;
    }
    let e = Env::new("options");
    let parent = e.launch_with(&e.folder, "probe", &["skip-permissions!"]);
    wait_until("parent report", || probe_args(&e.root).len() == 1);
    assert!(probe_args(&e.root)[0].contains(&"--fake-skip-permissions".to_string()));
    let view = |id: &str| {
        e.node()
            .supervisor
            .lock()
            .unwrap()
            .list()
            .into_iter()
            .find(|v| v.id.as_str() == id)
            .unwrap()
    };
    assert!(view(&parent).skips_permissions);
    assert_eq!(view(&parent).launch_options, ["skip-permissions"]);

    let mut m = e.mcp(&parent);
    // An extra argument naming the option is no way in either.
    let call = m.call(
        "agent_start",
        json!({"kind": "probe", "launch_options": ["skip-permissions"]}),
    );
    let p = e.request_of(&parent);
    e.decide(&p, true, 2, false).unwrap();
    let (v, is_error) = m.result(call);
    assert!(!is_error, "{v}");
    let helper = v["agent_id"].as_str().unwrap().to_string();
    wait_until("helper report", || probe_args(&e.root).len() == 2);
    let with_flag = probe_args(&e.root)
        .iter()
        .filter(|a| a.contains(&"--fake-skip-permissions".to_string()))
        .count();
    assert_eq!(with_flag, 1, "only the parent runs with the flag");
    assert!(!view(&helper).skips_permissions);
    assert!(view(&helper).launch_options.is_empty());
    let events = e.node().events.events_after(0);
    assert!(events.iter().any(|ev| matches!(&ev.body, Some(Body::AgentStarted(s)) if s.agent_id == parent && s.skips_permissions && s.launch_options == ["skip-permissions"])));
    assert!(events.iter().any(|ev| matches!(&ev.body, Some(Body::AgentStarted(s)) if s.agent_id == helper && !s.skips_permissions && s.launch_options.is_empty())));
}

/// ADR-0066: an agent's `agent_start` may carry settings. They are checked against the node's manifest before anyone is asked, shown on
/// the card (in the manifest's order, with the manifest's label), signed under `agora-approval-start-v2`, and applied exactly as shown;
/// a signature over anything else (the v1 message, other settings) starts nothing.
#[test]
fn a_helper_starts_with_the_settings_the_user_was_shown_and_signed() {
    if !supported() {
        return;
    }
    let e = Env::new("settings");
    let parent = e.launch(&e.folder, "probe");
    let mut m = e.mcp(&parent);
    // Refused before the user is asked: an undeclared setting, a value that would be a flag, a setting that is not a string.
    for (settings, code) in [
        (json!({"mode": "plan"}), "agent.setting_unknown"),
        // (six `agent_start` calls a minute are allowed, refused ones count; the value rules are tested in `launch_options`)
        (
            json!({"model": "--fake-skip-permissions"}),
            "agent.setting_invalid",
        ),
    ] {
        let id = m.call(
            "agent_start",
            json!({"kind": "probe", "settings": settings}),
        );
        assert_eq!(m.error_code(id), code, "{settings}");
    }
    let id = m.call(
        "agent_start",
        json!({"kind": "probe", "settings": {"model": 5}}),
    );
    assert_eq!(m.reply()["error"]["code"], -32602);
    let _ = id;
    assert!(
        e.node().approvals.waiting().is_empty(),
        "nothing was asked about"
    );

    // Asked in the manifest's order whatever order the call used; the card carries the labels; no grant is offered.
    let call = m.call(
        "agent_start",
        json!({"kind": "probe", "settings": {"model": "gpt-6.1-sol", "effort": "high"}}),
    );
    let p = e.request_of(&parent);
    let detail: Value = serde_json::from_str(&p.detail).unwrap();
    assert_eq!(
        detail["settings"],
        json!([
            {"id": "model", "label": "setting-model", "value": "gpt-6.1-sol"},
            {"id": "effort", "label": "setting-effort", "value": "high"},
        ])
    );
    assert_eq!(detail["grant_allowed"], false);
    let signed = agora_node::approvals::detail_settings(&p.detail);
    let v2 = |settings: &[(String, String)], detail: &str| {
        e.sign(&agora_node::approvals::start_signed_message_v2(
            NODE, &p.id, true, 0, false, settings, detail,
        ))
    };
    // The v1 message (a client that does not know settings), other settings, the same ones reordered or extended: all refused.
    let mut reordered = signed.clone();
    reordered.reverse();
    let mut changed = signed.clone();
    changed[0].1 = "gpt-6.1-terra".into();
    let mut extra = signed.clone();
    extra.push(("mode".into(), "plan".into()));
    let attempts = [
        e.sign(&start_signed_message(
            NODE, &p.id, true, 0, false, &p.detail,
        )),
        v2(&[], &p.detail),
        v2(&reordered, &p.detail),
        v2(&changed, &p.detail),
        v2(&extra, &p.detail),
        v2(&signed[..1], &p.detail),
        v2(&signed, &p.detail.replace("gpt-6.1-sol", "gpt-6.1-terra")),
    ];
    for sig in attempts {
        assert_eq!(
            e.resolve(&p, true, 0, false, sig).unwrap_err().code,
            "approval.signature_invalid"
        );
    }
    assert_eq!(probe_args(&e.root).len(), 1, "only the parent runs");
    // The decision over exactly what was shown starts the helper with exactly those arguments.
    e.resolve(&p, true, 0, false, v2(&signed, &p.detail))
        .unwrap();
    let (v, is_error) = m.result(call);
    assert!(!is_error, "{v}");
    wait_until("helper report", || probe_args(&e.root).len() == 2);
    let args = probe_args(&e.root);
    let helper_args = args
        .iter()
        .find(|a| a.contains(&"--fake-model".to_string()))
        .expect("the helper got the model");
    let at = helper_args
        .iter()
        .position(|a| a == "--fake-model")
        .unwrap();
    assert_eq!(
        helper_args[at..at + 4],
        ["--fake-model", "gpt-6.1-sol", "--fake-effort", "high"]
    );
    assert!(!helper_args.contains(&"--fake-skip-permissions".to_string()));
    assert_eq!(
        args.iter()
            .filter(|a| a.contains(&"--fake-model".to_string()))
            .count(),
        1,
        "the parent runs without it"
    );

    // A grant never covers a start with settings, and a start without them under a grant is unchanged.
    let call = m.call("agent_start", json!({"kind": "probe"}));
    let p2 = e.request_of(&parent);
    assert!(serde_json::from_str::<Value>(&p2.detail)
        .unwrap()
        .get("settings")
        .is_none());
    e.decide(&p2, true, 3, false).unwrap();
    let (v, is_error) = m.result(call);
    assert!(!is_error, "{v}");
    let call = m.call(
        "agent_start",
        json!({"kind": "probe", "settings": {"effort": "low"}}),
    );
    let p3 = e.request_of(&parent);
    assert_ne!(
        p3.id, p2.id,
        "asked again: the grant does not cover settings"
    );
    e.stop(&parent, true);
    let _ = call;
}

/// ADR-0066: the node, not the card, refuses a grant (and leave to ask in turn) on a request with settings: a decision that was signed
/// with `helper_grant` and `helpers_may_start` over such a request starts the helper and grants nothing, while one without settings
/// still grants.
#[test]
fn a_request_with_settings_is_never_granted_even_when_the_decision_says_so() {
    if !supported() {
        return;
    }
    let e = Env::new("settings-grant");
    let parent = e.launch(&e.folder, "probe");
    let mut m = e.mcp(&parent);
    let call = m.call(
        "agent_start",
        json!({"kind": "probe", "settings": {"effort": "low"}}),
    );
    let p = e.request_of(&parent);
    let signed = agora_node::approvals::detail_settings(&p.detail);
    let sig = e.sign(&agora_node::approvals::start_signed_message_v2(
        NODE, &p.id, true, 3, true, &signed, &p.detail,
    ));
    e.resolve(&p, true, 3, true, sig).unwrap();
    let (v, is_error) = m.result(call);
    assert!(!is_error, "{v}");
    let helper = v["agent_id"].as_str().unwrap().to_string();
    // No grant: the next start (without settings) is asked about, and the helper may not ask in turn.
    let call = m.call("agent_start", json!({"kind": "probe"}));
    let p2 = e.request_of(&parent);
    assert_ne!(p2.id, p.id);
    let mut h = e.mcp(&helper);
    let id = h.call("agent_start", json!({"kind": "probe"}));
    assert_eq!(h.error_code(id), "mesh.helper_limit");
    // A request without settings still grants, as before.
    e.decide(&p2, true, 3, false).unwrap();
    let (v, is_error) = m.result(call);
    assert!(!is_error, "{v}");
    let call = m.call("agent_start", json!({"kind": "probe"}));
    let (v, is_error) = m.result(call);
    assert!(!is_error, "started under the grant without asking: {v}");
    e.stop(&parent, true);
}

/// ADR-0059 decision 5: an agent's `agent_start` may name launch options, so a helper acts without asking, but only when the user's
/// signed decision names them. They are checked against the node's manifest and the owner's policy before anyone is asked, shown on the
/// card with whether each skips permissions, signed under `agora-approval-start-v3`, never covered by a grant, and the helper carries
/// the "does not ask" badge.
#[test]
fn a_helper_acts_without_asking_only_when_the_signed_decision_names_it() {
    if !supported() {
        return;
    }
    let e = Env::new("helper-options");
    let parent = e.launch(&e.folder, "probe");
    let mut m = e.mcp(&parent);
    // Refused before the user is asked: an option the kind does not declare, one that is not a string, one the owner forbids.
    let id = m.call(
        "agent_start",
        json!({"kind": "probe", "options": ["no-such-option"]}),
    );
    assert_eq!(m.error_code(id), "agent.option_invalid");
    let _ = m.call("agent_start", json!({"kind": "probe", "options": [5]}));
    assert_eq!(m.reply()["error"]["code"], -32602);
    e.node()
        .supervisor
        .lock()
        .unwrap()
        .set_skip_permissions(agora_node::launch_options::SkipPermissions::Forbid);
    let id = m.call(
        "agent_start",
        json!({"kind": "probe", "options": ["skip-permissions"]}),
    );
    assert_eq!(m.error_code(id), "agent.option_forbidden");
    assert!(
        e.node().approvals.waiting().is_empty(),
        "nothing was asked about"
    );
    e.node()
        .supervisor
        .lock()
        .unwrap()
        .set_skip_permissions(agora_node::launch_options::SkipPermissions::Allow);

    // The card names the option and that it skips permissions; no grant is offered.
    let call = m.call(
        "agent_start",
        json!({"kind": "probe", "options": ["skip-permissions"], "settings": {"effort": "high"}}),
    );
    let p = e.request_of(&parent);
    let detail: Value = serde_json::from_str(&p.detail).unwrap();
    assert_eq!(
        detail["launch_options"],
        json!([{"id": "skip-permissions", "label": "option-skip-permissions", "skips_permissions": true}])
    );
    assert_eq!(detail["grant_allowed"], false);
    let settings = agora_node::approvals::detail_settings(&p.detail);
    let options = agora_node::approvals::detail_launch_options(&p.detail);
    assert_eq!(options, ["skip-permissions"]);
    let v3 = |settings: &[(String, String)], options: &[String], detail: &str| {
        e.sign(&agora_node::approvals::start_signed_message_v3(
            NODE, &p.id, true, 0, false, settings, options, detail,
        ))
    };
    // What a client that does not know options signs (v1, v2), and v3 over anything but what was shown: all refused.
    let attempts = [
        e.sign(&start_signed_message(
            NODE, &p.id, true, 0, false, &p.detail,
        )),
        e.sign(&agora_node::approvals::start_signed_message_v2(
            NODE, &p.id, true, 0, false, &settings, &p.detail,
        )),
        v3(&settings, &[], &p.detail),
        v3(&[], &options, &p.detail),
        v3(
            &settings,
            &["skip-permissions".into(), "x".into()],
            &p.detail,
        ),
        v3(
            &settings,
            &options,
            &p.detail
                .replace("\"skips_permissions\":true", "\"skips_permissions\":false"),
        ),
    ];
    for sig in attempts {
        assert_eq!(
            e.resolve(&p, true, 0, false, sig).unwrap_err().code,
            "approval.signature_invalid"
        );
    }
    assert_eq!(probe_args(&e.root).len(), 1, "only the parent runs");
    // The decision over exactly what was shown starts the helper with the option; a grant signed with it is ignored.
    let sig = e.sign(&agora_node::approvals::start_signed_message_v3(
        NODE, &p.id, true, 3, true, &settings, &options, &p.detail,
    ));
    e.resolve(&p, true, 3, true, sig).unwrap();
    let (v, is_error) = m.result(call);
    assert!(!is_error, "{v}");
    let helper = v["agent_id"].as_str().unwrap().to_string();
    wait_until("helper report", || probe_args(&e.root).len() == 2);
    let with_flag: Vec<_> = probe_args(&e.root)
        .into_iter()
        .filter(|a| a.contains(&"--fake-skip-permissions".to_string()))
        .collect();
    assert_eq!(with_flag.len(), 1, "only the helper runs with the flag");
    assert!(with_flag[0].contains(&"--fake-effort".to_string()));
    let view = e
        .node()
        .supervisor
        .lock()
        .unwrap()
        .list()
        .into_iter()
        .find(|v| v.id.as_str() == helper)
        .unwrap();
    assert!(view.skips_permissions);
    assert_eq!(view.launch_options, ["skip-permissions"]);
    let events = e.node().events.events_after(0);
    assert!(events.iter().any(|ev| matches!(&ev.body, Some(Body::AgentStarted(s)) if s.agent_id == helper && s.skips_permissions && s.launch_options == ["skip-permissions"] && s.parent_agent_id == parent)));
    // No grant came with it: the next start, with or without the option, is asked about, and the helper may not ask in turn.
    let mut h = e.mcp(&helper);
    let id = h.call("agent_start", json!({"kind": "probe"}));
    assert_eq!(h.error_code(id), "mesh.helper_limit");
    let call = m.call(
        "agent_start",
        json!({"kind": "probe", "options": ["skip-permissions"]}),
    );
    let p2 = e.request_of(&parent);
    assert_ne!(p2.id, p.id, "asked again: no grant covers an option");
    e.stop(&parent, true);
    let _ = call;
}

/// #1147 (audit of #1157): a value the node holds, in the caller's prompt or its folder's name, never reaches the helper-start card
/// (the queue, the hub's event, the window): the detail the start signature binds holds it replaced, so a decision over what the card
/// showed still starts the helper.
#[test]
fn a_secret_in_a_helper_starts_prompt_never_reaches_its_card() {
    if !supported() {
        return;
    }
    // Letters only: the card's folder path holds the test's process id, whose digits could match a piece of a value with a digit run.
    let value = "SK-HELPER-qwertyuiop-ABCDEFGHIJ";
    let e = Env::with_secret("secret", Some(value));
    let folder = e.folder.join(format!("dir-{value}"));
    std::fs::create_dir_all(&folder).unwrap();
    let parent = e.launch(&e.folder, "probe");
    let mut m = e.mcp(&parent);
    let call = m.call(
        "agent_start",
        json!({"kind": "screen", "prompt": format!("use {value} to log in"), "folder": folder}),
    );
    let p = e.request_of(&parent);
    let pieces: Vec<&str> = (0..=value.len() - 4).map(|i| &value[i..i + 4]).collect();
    let leaks = |t: &str| pieces.iter().filter(|x| t.contains(**x)).count();
    assert_eq!(leaks(&p.detail), 0, "{}", p.detail);
    let detail: Value = serde_json::from_str(&p.detail).unwrap();
    assert_eq!(detail["prompt"], "use <redacted> to log in");
    let events = e.node().events.events_after(0);
    let shown: Vec<String> = events
        .iter()
        .filter_map(|ev| match &ev.body {
            Some(Body::ApprovalRequested(a)) => Some(a.detail.clone()),
            _ => None,
        })
        .collect();
    assert!(!shown.is_empty());
    assert!(shown.iter().all(|d| leaks(d) == 0), "{shown:?}");
    // The decision signs what the card showed; the helper starts.
    e.decide(&p, true, 0, false).unwrap();
    let (v, is_error) = m.result(call);
    assert!(!is_error, "{v}");
}
