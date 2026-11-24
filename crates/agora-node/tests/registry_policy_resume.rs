//! #1133 review F3: the owner's `agents.registry` policy also stops a Resume of an agent of a registry kind, for a structured agent
//! (an adapter, as the registry's ACP agents are) and for a terminal one. The policy is applied by origin (`register_from_registry`),
//! so the same kind registered by hand resumes. The adapter is `agora-fake-agent --adapter`; never a real agent.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "private_home/mod.rs"]
mod private_home;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use agora_adapters::Manifest;
use agora_core::NodeId;
use agora_node::manifests::RegistryPolicy;
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, StartRequest, Supervisor, SupervisorConfig, SupervisorError,
};
use agora_node::terminal::TerminalOptions;

fn fake_agent() -> String {
    let mut p = std::env::current_exe().expect("exe");
    p.pop();
    p.pop();
    p.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    static FRESH: std::sync::Once = std::sync::Once::new();
    FRESH.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let ok = Command::new(cargo)
            .args(["build", "-q", "-p", "agora-fake-agent"])
            .status()
            .expect("run cargo")
            .success();
        assert!(ok, "building agora-fake-agent failed");
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

fn repo(name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("agora-regres-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    (dir, repo)
}

fn supervisor() -> Supervisor {
    Supervisor::new(SupervisorConfig {
        node: NodeId::new("node-1"),
        restart: RestartPolicy {
            max_restarts: 0,
            ..RestartPolicy::default()
        },
        terminal: TerminalOptions::default(),
        limits: NodeLimits::default(),
    })
}

fn adapter(kind: &str) -> Manifest {
    Manifest::parse(&format!(
        "kind = \"{kind}\"\ndisplay_name = \"F\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\"]\n",
        fake_agent()
    ))
    .unwrap()
}

fn terminal(kind: &str) -> Manifest {
    Manifest::parse(&format!(
        "kind = \"{kind}\"\ndisplay_name = \"T\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n"
    ))
    .unwrap()
}

fn start(
    s: &mut Supervisor,
    dir: &Path,
    repo: &Path,
    kind: &str,
    name: &str,
) -> agora_core::AgentId {
    s.start(
        &StartRequest {
            kind: kind.into(),
            checkout: repo.to_path_buf(),
            worktree_root: dir.join("worktrees"),
            name: name.into(),
            base: None,
        },
        Instant::now(),
    )
    .expect("start")
}

fn rule(e: SupervisorError) -> String {
    match e {
        SupervisorError::RegistryRefused(_, rule) => rule.to_string(),
        other => panic!("not a registry refusal: {other:?}"),
    }
}

#[test]
fn a_registry_kind_the_policy_switches_off_is_not_resumed_and_a_hand_made_one_is() {
    private_home::init();
    let mut kinds = vec![("registry-fake-adapter", adapter as fn(&str) -> Manifest)];
    if agora_platform::PTY_SUPPORTED {
        kinds.push(("registry-fake-terminal", terminal));
    }
    for (kind, make) in kinds {
        for (policy, want) in [
            (
                RegistryPolicy {
                    off: true,
                    ..Default::default()
                },
                "registry_off",
            ),
            (
                RegistryPolicy {
                    off: false,
                    deny: [kind.to_string()].into(),
                },
                "registry_denied",
            ),
        ] {
            let (dir, repo) = repo(&format!("{kind}-{want}"));
            let mut s = supervisor();
            s.register_from_registry(make(kind));
            let id = start(&mut s, &dir, &repo, kind, "first");
            s.stop(&id).unwrap();
            // The owner switches the registry's kinds off (the node reads its policy at start; here, set directly).
            s.set_registry_policy(policy.clone());
            assert_eq!(
                rule(s.resume(&id, Instant::now()).unwrap_err()),
                want,
                "{kind}"
            );
            assert!(
                s.kinds().iter().all(|k| k.kind != kind),
                "{kind} is not offered"
            );
            // The same kind described by hand is the owner's own: the policy does not touch it.
            s.register(make(kind));
            s.resume(&id, Instant::now())
                .unwrap_or_else(|e| panic!("{kind}: {e:?}"));
            s.stop(&id).unwrap();
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
