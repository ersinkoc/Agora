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
    for (kind, extra) in [("acp-fake", ""), ("acp-broken", ", \"--fail-at-start\"")] {
        std::fs::write(
            home.join(format!("manifests/{kind}.toml")),
            format!("kind = \"{kind}\"\ndisplay_name = \"Fake ACP\"\nversion = \"1\"\nmin_engine = 1\n[structured]\nacp = {{ argv = [{:?}, \"--acp\"{extra}] }}\n", fake_agent()),
        )
        .unwrap();
    }
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
    Some((home, node))
}

fn started_agent(home: &Path) -> String {
    let (code, started) = agora(home, &["agent", "start", "acp-fake"]);
    assert_eq!(code, 0, "{started}");
    started
        .split_whitespace()
        .find(|w| w.starts_with("agent-"))
        .expect("an agent id")
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '-')
        .to_string()
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
