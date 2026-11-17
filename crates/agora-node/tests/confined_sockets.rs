//! ADR-0083, step 1: a confined agent's environment is scrubbed of the variables that point at the user's sockets (ssh-agent, gpg-agent,
//! the session bus, docker, tmux), in a real spawn. The owner lets one through with `sandbox.pass_env`; a manifest's own `[process]
//! env` still wins; an agent that is not confined keeps them. Negative controls throughout: the same node environment, no scrub.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use agora_adapters::Manifest;
use agora_core::NodeId;
use agora_node::sandbox::{parse_pass_env, Level, SandboxPolicy};
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, StartRequest, Supervisor, SupervisorConfig,
};
use agora_node::terminal::TerminalOptions;

const NAMES: [&str; 6] = [
    "SSH_AUTH_SOCK",
    "GPG_AGENT_INFO",
    "DBUS_SESSION_BUS_ADDRESS",
    "DOCKER_HOST",
    "TMUX",
    "TMUX_PANE",
];

fn git(dir: &Path, args: &[&str]) {
    assert!(std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .unwrap()
        .success());
}

/// Starts a shell agent under `policy` whose manifest has `extra` in `[process]`, and returns what it printed about the variables.
fn env_seen(policy: SandboxPolicy, extra: &str, tag: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("sockenv-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let probe: String = NAMES
        .iter()
        .map(|n| format!("echo \"{n}=${{{n}-unset}}\";"))
        .collect();
    let script = format!("{{ {probe} echo done; }} > out 2>&1; sleep 30");
    let manifest = Manifest::parse(&format!(
        "kind = \"shy\"\ndisplay_name = \"Shy\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"/bin/sh\", \"-c\", {script:?}]\n{extra}"
    ))
    .unwrap();
    let mut sup = Supervisor::new(SupervisorConfig {
        node: NodeId::new("node-1"),
        restart: RestartPolicy::default(),
        terminal: TerminalOptions::default(),
        limits: NodeLimits::default(),
    });
    sup.register(manifest);
    sup.set_sandbox(policy);
    let req = StartRequest {
        kind: "shy".into(),
        checkout: repo,
        worktree_root: dir.join("worktrees"),
        name: "a".into(),
        base: None,
    };
    sup.start(&req, Instant::now()).unwrap();
    let out = dir.join("worktrees/a/out");
    let end = Instant::now() + Duration::from_secs(15);
    let read = || std::fs::read_to_string(&out).unwrap_or_default();
    while !read().contains("done") && Instant::now() < end {
        sup.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(50));
    }
    let text = read();
    drop(sup);
    let _ = std::fs::remove_dir_all(dir);
    assert!(text.contains("done"), "the agent did not finish: {text}");
    text
}

#[test]
fn a_confined_agent_does_not_inherit_socket_variables_and_the_owner_can_pass_one() {
    if !agora_platform::sandbox_supported() || !agora_platform::PTY_SUPPORTED {
        eprintln!("skipped: no confinement or no PTY on this machine");
        return;
    }
    // The node's own environment (this process), as a node started from a login shell has it.
    for n in NAMES {
        std::env::set_var(n, format!("/node/{n}"));
    }
    let confined = |pass: &str| SandboxPolicy {
        level: Level::Worktree,
        pass_env: parse_pass_env(pass),
        ..Default::default()
    };

    // Negative control: not confined, the agent has every one of them.
    let text = env_seen(SandboxPolicy::default(), "", "off");
    for n in NAMES {
        assert!(text.contains(&format!("{n}=/node/{n}")), "{n}: {text}");
    }

    // Confined: none of them.
    let text = env_seen(confined(""), "", "scrub");
    for n in NAMES {
        assert!(text.contains(&format!("{n}=unset")), "{n}: {text}");
        assert!(!text.contains("/node/"), "{text}");
    }

    // The owner passes one by name: that one only.
    let text = env_seen(confined("SSH_AUTH_SOCK"), "", "pass");
    assert!(text.contains("SSH_AUTH_SOCK=/node/SSH_AUTH_SOCK"), "{text}");
    assert!(text.contains("DOCKER_HOST=unset"), "{text}");
    assert!(text.contains("TMUX=unset"), "{text}");

    // A malformed list passes nothing at all.
    let text = env_seen(confined("SSH_AUTH_SOCK, TMUX=1"), "", "malformed");
    assert!(text.contains("SSH_AUTH_SOCK=unset"), "{text}");

    // A manifest's own value is the agent's own choice and still wins.
    let text = env_seen(
        confined(""),
        "[process.env]\nDOCKER_HOST = \"unix:///agent/own\"\n",
        "manifest",
    );
    assert!(text.contains("DOCKER_HOST=unix:///agent/own"), "{text}");
    assert!(text.contains("SSH_AUTH_SOCK=unset"), "{text}");
    for n in NAMES {
        std::env::remove_var(n);
    }
}
