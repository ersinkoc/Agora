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
    let end = Instant::now() + Duration::from_secs(15);
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
