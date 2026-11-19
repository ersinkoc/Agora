//! An ACP (L2) agent through a real node: the manifest says `[structured] acp`, the node runs `agora acp-bridge` around the agent,
//! and everything else (start, prompt, transcript) works as for any chat agent. The agent is the fake ACP agent, never a real one.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use agora_node::control::{Client, KIND_CHAT_ENTRY, KIND_CHAT_SNAPSHOT};
use agora_protocol::v1::{self, command_request::Command as Cmd};
use prost::Message;

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

fn fake_agent() -> String {
    let mut p = std::path::PathBuf::from(AGORA);
    p.pop();
    p.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    // Built once per test binary, even when present: a binary left from an older checkout may lack newer flags.
    static FRESH: std::sync::Once = std::sync::Once::new();
    FRESH.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        assert!(
            Command::new(cargo)
                .args(["build", "-q", "-p", "agora-fake-agent"])
                .status()
                .expect("run cargo")
                .success(),
            "building agora-fake-agent failed"
        );
    });
    p.to_string_lossy().into_owned()
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .expect("git")
        .success();
    assert!(ok, "git {args:?}");
}

struct Node(std::process::Child);
impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn agora(home: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(AGORA)
        .args(args)
        .current_dir(home.join("demo"))
        .env("AGORA_HOME", home)
        .env("AGORA_LANG", "en")
        .output()
        .expect("run agora");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr),
    )
}

#[test]
fn an_acp_agent_runs_through_the_bridge_like_any_chat_agent() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let home = std::env::temp_dir().join(format!("agora-acp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let repo = home.join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    std::fs::create_dir_all(home.join("manifests")).unwrap();
    std::fs::write(
        home.join("manifests/acp-fake.toml"),
        format!("kind = \"acp-fake\"\ndisplay_name = \"Fake ACP\"\nversion = \"1\"\nmin_engine = 1\n[structured]\nacp = {{ argv = [{:?}, \"--acp\"] }}\n", fake_agent()),
    )
    .unwrap();
    let node = Node(
        Command::new(AGORA)
            .args(["node", "run", "--project"])
            .arg(&repo)
            .env("AGORA_HOME", &home)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start node"),
    );
    let sock = home.join("node.sock");
    let end = Instant::now() + Duration::from_secs(60);
    while !sock.exists() {
        assert!(Instant::now() < end, "node did not create its socket");
        std::thread::sleep(Duration::from_millis(50));
    }

    let (code, kinds) = agora(&home, &["agent", "kinds"]);
    assert!(code == 0 && kinds.contains("acp-fake"), "{kinds}");
    let (code, started) = agora(&home, &["agent", "start", "acp-fake"]);
    assert_eq!(code, 0, "{started}");
    let id = started
        .split_whitespace()
        .find(|w| w.starts_with("agent-"))
        .expect("an agent id")
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '-')
        .to_string();
    wait_ready(&home, &id);

    let mut c = Client::connect(&sock).unwrap();
    c.call_on(
        3,
        Cmd::ChatSubscribe(v1::ChatSubscribe {
            agent_id: id.clone(),
        }),
    )
    .unwrap()
    .expect("subscribed");
    let (code, out) = agora(&home, &["agent", "prompt", &id, "plan"]);
    assert_eq!(code, 0, "{out}");

    let end = Instant::now() + Duration::from_secs(15);
    let mut kinds_seen = Vec::new();
    let mut texts = Vec::new();
    while Instant::now() < end && !texts.iter().any(|t: &String| t == "hello world") {
        let Some(f) = c.read_frame(Some(Duration::from_millis(200))).unwrap() else {
            continue;
        };
        if f.stream_id != 3 {
            continue;
        }
        let entries = match f.kind {
            KIND_CHAT_SNAPSHOT => {
                v1::ChatSnapshot::decode(f.payload.as_slice())
                    .unwrap()
                    .entries
            }
            KIND_CHAT_ENTRY => vec![v1::ChatEntry::decode(f.payload.as_slice()).unwrap()],
            _ => continue,
        };
        for e in entries {
            kinds_seen.push(e.kind);
            texts.push(e.text);
        }
    }
    assert!(
        texts.iter().any(|t| t == "hello world"),
        "the agent's message arrived joined: {texts:?}"
    );
    assert!(
        kinds_seen.contains(&(v1::ChatKind::Plan as i32))
            && kinds_seen.contains(&(v1::ChatKind::Diff as i32))
            && kinds_seen.contains(&(v1::ChatKind::ToolCall as i32)),
        "{kinds_seen:?}"
    );
    // The state the bridge reports (idle once the turn ended) reaches `agent list`; it used to stay `unknown`.
    let end = Instant::now() + Duration::from_secs(15);
    let listed = loop {
        let (_, out) = agora(&home, &["agent", "list"]);
        if out.contains("idle") || Instant::now() > end {
            break out;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(listed.contains("idle"), "{listed}");
    drop(node);
    let _ = std::fs::remove_dir_all(&home);
}

/// A node with two ACP kinds: `acp-fake` (the fake agent) and `acp-broken` (one that dies before `initialize`).
fn node_with_acp(tag: &str) -> Option<(std::path::PathBuf, Node)> {
    node_with_acp_policy(tag, None)
}

/// Like [`node_with_acp`]; with `policy` the node's policy file holds it (the kind `acp-bypass` offers a mode that skips permissions).
fn node_with_acp_policy(tag: &str, policy: Option<&str>) -> Option<(std::path::PathBuf, Node)> {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return None;
    }
    let home = std::env::temp_dir().join(format!("agora-acp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let repo = home.join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    std::fs::create_dir_all(home.join("manifests")).unwrap();
    for (kind, extra) in [
        ("acp-fake", ""),
        ("acp-broken", ", \"--fail-at-start\""),
        ("acp-bypass", ", \"--bypass-mode\""),
    ] {
        std::fs::write(
            home.join(format!("manifests/{kind}.toml")),
            format!("kind = \"{kind}\"\ndisplay_name = \"Fake ACP\"\nversion = \"1\"\nmin_engine = 1\n[structured]\nacp = {{ argv = [{:?}, \"--acp\"{extra}] }}\n", fake_agent()),
        )
        .unwrap();
    }
    if let Some(policy) = policy {
        std::fs::create_dir_all(home.join("data")).unwrap();
        std::fs::write(home.join("data/policy"), policy).unwrap();
    }
    let mut cmd = Command::new(AGORA);
    cmd.args(["node", "run", "--project"])
        .arg(&repo)
        .env("AGORA_HOME", &home)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let node = Node(cmd.spawn().expect("start node"));
    let sock = home.join("node.sock");
    let end = Instant::now() + Duration::from_secs(60);
    while !sock.exists() {
        assert!(Instant::now() < end, "node did not create its socket");
        std::thread::sleep(Duration::from_millis(50));
    }
    Some((home, node))
}

fn started_agent(home: &Path) -> String {
    started_agent_of(home, "acp-fake")
}

fn started_agent_of(home: &Path, kind: &str) -> String {
    let (code, started) = agora(home, &["agent", "start", kind]);
    assert_eq!(code, 0, "{started}");
    let id = started
        .split_whitespace()
        .find(|w| w.starts_with("agent-"))
        .expect("an agent id")
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '-')
        .to_string();
    wait_ready(home, &id);
    id
}

/// Waits until the bridge has its session (the agent is `idle`): `agent start` answers once the process runs, and a prompt sent
/// before the session exists is refused ("no session"), so a slow runner (seen on Windows CI) lost the prompt and its answer.
fn wait_ready(home: &Path, id: &str) {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let (_, out) = agora(home, &["agent", "list"]);
        if out.lines().any(|l| l.contains(id) && l.contains("idle")) {
            return;
        }
        assert!(Instant::now() < end, "{id} never became idle: {out}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// ADR-0058: a turn that ends in silence is a notice in the conversation, with a code and no text.
#[test]
fn a_silent_turn_reaches_the_conversation_as_a_notice() {
    let Some((home, node)) = node_with_acp("notice") else {
        return;
    };
    let id = started_agent(&home);
    let mut c = Client::connect(&home.join("node.sock")).unwrap();
    c.call_on(
        3,
        Cmd::ChatSubscribe(v1::ChatSubscribe {
            agent_id: id.clone(),
        }),
    )
    .unwrap()
    .expect("subscribed");
    let (code, out) = agora(&home, &["agent", "prompt", &id, "silent"]);
    assert_eq!(code, 0, "{out}");
    let end = Instant::now() + Duration::from_secs(15);
    let notice = loop {
        assert!(Instant::now() < end, "no notice arrived");
        let Some(f) = c.read_frame(Some(Duration::from_millis(200))).unwrap() else {
            continue;
        };
        if f.stream_id != 3 {
            continue;
        }
        let entries = match f.kind {
            KIND_CHAT_SNAPSHOT => {
                v1::ChatSnapshot::decode(f.payload.as_slice())
                    .unwrap()
                    .entries
            }
            KIND_CHAT_ENTRY => vec![v1::ChatEntry::decode(f.payload.as_slice()).unwrap()],
            _ => continue,
        };
        if let Some(n) = entries
            .into_iter()
            .find(|e| e.kind == v1::ChatKind::Notice as i32)
        {
            break n;
        }
    };
    assert_eq!(notice.code, "adapter.turn_empty");
    assert!(notice.text.is_empty());
    drop(node);
    let _ = std::fs::remove_dir_all(&home);
}

/// ADR-0058: the user switches the agent's mode and model among what it offered; anything else is refused before it reaches the
/// agent.
#[test]
fn the_user_switches_an_acp_agents_mode_and_model() {
    let Some((home, node)) = node_with_acp("modes") else {
        return;
    };
    let id = started_agent(&home);
    let mut c = Client::connect(&home.join("node.sock")).unwrap();
    let set_mode = |c: &mut Client, mode: &str| {
        c.call(Cmd::AgentSetMode(v1::AgentSetMode {
            agent_id: id.clone(),
            mode: mode.into(),
        }))
        .unwrap()
    };
    // The options arrive just after the start; until then nothing is offered.
    let end = Instant::now() + Duration::from_secs(15);
    while set_mode(&mut c, "plan").is_err() {
        assert!(Instant::now() < end, "the mode was never accepted");
        std::thread::sleep(Duration::from_millis(100));
    }
    let refused = set_mode(&mut c, "nope").unwrap_err();
    assert_eq!(refused.code, "protocol.invalid_message");
    let model = |c: &mut Client, model: &str| {
        c.call(Cmd::AgentSetModel(v1::AgentSetModel {
            agent_id: id.clone(),
            model: model.into(),
        }))
        .unwrap()
    };
    assert!(model(&mut c, "fake/model-2").is_ok());
    assert_eq!(
        model(&mut c, "fake/model-9").unwrap_err().code,
        "protocol.invalid_message"
    );
    let unknown = c
        .call(Cmd::AgentSetModel(v1::AgentSetModel {
            agent_id: "agent-99".into(),
            model: "fake/model-2".into(),
        }))
        .unwrap()
        .unwrap_err();
    assert_eq!(unknown.code, "agent.not_found");
    drop(node);
    let _ = std::fs::remove_dir_all(&home);
}

/// A mode in which the agent acts without asking follows the node's `skip_permissions` policy, as a launch option does (ADR-0059):
/// `forbid` refuses it with `agent.option_forbidden`, `allow` takes it; a mode that asks stays possible under `forbid`.
#[test]
fn a_mode_that_skips_permissions_follows_the_nodes_policy() {
    for (policy, allowed) in [
        ("skip_permissions = forbid\n", false),
        ("skip_permissions = allow\n", true),
    ] {
        let tag = if allowed { "skip-allow" } else { "skip-forbid" };
        let Some((home, node)) = node_with_acp_policy(tag, Some(policy)) else {
            return;
        };
        let id = started_agent_of(&home, "acp-bypass");
        let mut c = Client::connect(&home.join("node.sock")).unwrap();
        let mut set_mode = |mode: &str| {
            c.call(Cmd::AgentSetMode(v1::AgentSetMode {
                agent_id: id.clone(),
                mode: mode.into(),
            }))
            .unwrap()
        };
        // The options arrive just after the start.
        let end = Instant::now() + Duration::from_secs(15);
        while set_mode("plan").is_err() {
            assert!(Instant::now() < end, "the mode was never accepted");
            std::thread::sleep(Duration::from_millis(100));
        }
        let skipped = set_mode("bypassPermissions");
        if allowed {
            assert!(skipped.is_ok(), "{skipped:?}");
        } else {
            let err = skipped.unwrap_err();
            assert_eq!(err.code, "agent.option_forbidden");
            assert!(set_mode("build").is_ok());
        }
        drop(node);
        let _ = std::fs::remove_dir_all(&home);
    }
}

/// ADR-0090 (schema only): no node handles `AgentSetConfig` or `AgentCancelTurn` yet, so each is answered with
/// `protocol.capability_missing`, never accepted and never dropped.
#[test]
fn the_new_conversation_commands_are_refused_until_a_node_handles_them() {
    let Some((home, node)) = node_with_acp("refuse-0090") else {
        return;
    };
    let id = started_agent(&home);
    let mut c = Client::connect(&home.join("node.sock")).unwrap();
    for (cmd, capability) in [
        (
            Cmd::AgentSetConfig(v1::AgentSetConfig {
                agent_id: id.clone(),
                config_id: "effort".into(),
                value: "high".into(),
            }),
            "agent.set_config",
        ),
        (
            Cmd::AgentCancelTurn(v1::AgentCancelTurn {
                agent_id: id.clone(),
                drop_queue: false,
            }),
            "agent.cancel_turn",
        ),
    ] {
        let err = c.call(cmd).unwrap().unwrap_err();
        assert_eq!(err.code, "protocol.capability_missing");
        assert_eq!(
            err.params.get("capability").and_then(|p| p.value.clone()),
            Some(v1::param_value::Value::Str(capability.into()))
        );
    }
    drop(node);
    let _ = std::fs::remove_dir_all(&home);
}

/// ADR-0058: an agent that dies while starting is reported with its reason, not as `internal`.
#[test]
fn an_acp_agent_that_dies_while_starting_says_why() {
    let Some((home, node)) = node_with_acp("dies") else {
        return;
    };
    let mut c = Client::connect(&home.join("node.sock")).unwrap();
    let projects = match c
        .call(Cmd::ProjectList(v1::ProjectList {}))
        .unwrap()
        .unwrap()
        .result
    {
        Some(v1::command_ok::Result::ProjectList(p)) => p.projects,
        other => panic!("{other:?}"),
    };
    let err = c
        .call(Cmd::AgentStart(v1::AgentStart {
            project_id: projects[0].project_id.clone(),
            kind: "acp-broken".into(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap_err();
    assert_eq!(err.code, "adapter.agent_exited", "{err:?}");
    let param = |k: &str| match err.params.get(k).and_then(|p| p.value.clone()) {
        Some(v1::param_value::Value::Str(s)) => s,
        other => panic!("{k}: {other:?}"),
    };
    assert_eq!(param("stage"), "initialize");
    assert_eq!(param("exit_code"), "5");
    let tail = param("stderr_tail");
    assert!(tail.contains("plugin failed"), "{tail}");
    assert!(!tail.contains("sk-fake"), "a secret leaked: {tail}");
    // The command line shows it in words, from the locale files.
    let (code, out) = agora(&home, &["agent", "start", "acp-broken"]);
    assert_ne!(code, 0);
    assert!(out.contains("exited before it was ready"), "{out}");
    drop(node);
    let _ = std::fs::remove_dir_all(&home);
}

/// Prompts `agent` and returns the first thing it says (after the prompt) that `want` accepts.
fn answer(home: &Path, agent: &str, prompt: &str, want: impl Fn(&str) -> bool) -> String {
    let mut c = Client::connect(&home.join("node.sock")).unwrap();
    c.call_on(
        3,
        Cmd::ChatSubscribe(v1::ChatSubscribe {
            agent_id: agent.into(),
        }),
    )
    .unwrap()
    .expect("subscribed");
    let (code, out) = agora(home, &["agent", "prompt", agent, prompt]);
    assert_eq!(code, 0, "{out}");
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        assert!(Instant::now() < end, "{agent} did not answer {prompt}");
        let Some(f) = c.read_frame(Some(Duration::from_millis(200))).unwrap() else {
            continue;
        };
        if f.stream_id != 3 || f.kind != KIND_CHAT_ENTRY {
            continue;
        }
        let e = v1::ChatEntry::decode(f.payload.as_slice()).unwrap();
        if e.role == v1::ChatRole::Agent as i32 && want(&e.text) {
            return e.text;
        }
    }
}

/// ACP agents get the mesh: the node passes the agent's own mesh server through the bridge into ACP `session/new`, the agent starts it
/// (`agora mesh stdio --socket ...`, a real MCP conversation), sees the other agents and is seen by them, and its `agent_start` waits for
/// the user's decision like any agent's.
#[test]
fn an_acp_agent_uses_the_mesh_through_its_own_server() {
    let Some((home, node)) = node_with_acp("mesh") else {
        return;
    };
    let first = started_agent(&home);
    let second = started_agent(&home);

    let said = answer(&home, &first, "mcp-servers", |t| t.starts_with('['));
    let servers: serde_json::Value = serde_json::from_str(&said).unwrap();
    assert_eq!(servers.as_array().map(Vec::len), Some(1), "{servers}");
    assert_eq!(servers[0]["name"], "agora-mesh");
    assert_eq!(
        std::fs::canonicalize(servers[0]["command"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(AGORA).unwrap(),
        "the node's own program"
    );
    let args: Vec<&str> = servers[0]["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert_eq!(args[..3], ["mesh", "stdio", "--socket"]);
    assert!(
        args[3].ends_with(&format!("{first}.sock")),
        "its own socket: {args:?}"
    );
    assert_eq!(servers[0]["env"], serde_json::json!([]));

    // Each sees the other through its own mesh server.
    let list = answer(&home, &second, "mcp agents_list {}", |t| {
        t.contains("agent_id")
    });
    assert!(
        list.contains(&format!("\"agent_id\":\"{first}\""))
            && list.contains("\"kind\":\"acp-fake\""),
        "{list}"
    );

    // A start it asks for waits for the user (no trusted device here, so nobody can answer it yet); while it waits, a second one is
    // refused as `pending`: the first reached the approval queue.
    let ask = r#"mcp agent_start {"kind":"acp-fake","prompt":"hello"}"#;
    let first_try = answer(&home, &first, ask, |t| {
        !t.starts_with('[') && !t.contains("agent_id")
    });
    assert_eq!(first_try, "waiting");
    let second_try = answer(&home, &first, ask, |t| t.starts_with("error"));
    assert!(
        second_try.contains("mesh.helper_limit") && second_try.contains("pending"),
        "{second_try}"
    );
    drop(node);
    let _ = std::fs::remove_dir_all(&home);
}
