//! ADR-0060, `[resume] fresh_argv`: an agent whose tool wrote no session until it was spoken to (Claude Code) has nothing to resume
//! after a node restart. A resume that fails within moments starts the session from nothing under the same id, for a kind whose
//! manifest says how; otherwise nothing changes. The "tool" is `agora-fake-agent` (never a real agent): with an unknown flag it exits
//! at once with a failure, and `--probe-directory <file>` writes that file.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use agora_adapters::Manifest;
use agora_core::NodeId;
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, Restore, StartRequest, Supervisor, SupervisorConfig,
};
use agora_node::terminal::TerminalOptions;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const SID: &str = "11111111-2222-4333-8444-555555555555";

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
            "building the fake failed"
        );
    });
    p.to_string_lossy().into_owned()
}

fn git(dir: &Path, args: &[&str]) {
    assert!(Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .unwrap()
        .success());
}

struct Env {
    root: PathBuf,
    repo: PathBuf,
}

impl Env {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("fresh-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "x").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        Self { root, repo }
    }

    fn report(&self) -> PathBuf {
        self.root.join(format!("fresh-{SID}.json"))
    }

    /// A supervisor with a kind `k`: it launches and resumes as a command that fails at once; `fresh` says whether its manifest
    /// has a `fresh_argv` (which writes the report file, named with the session id).
    fn supervisor(&self, fresh: bool) -> Supervisor {
        let fake = serde_json::to_string(&fake()).unwrap();
        let report = serde_json::to_string(&self.report().to_string_lossy()).unwrap();
        let report = report.replace(SID, "{session_id}");
        let fresh = if fresh {
            format!("fresh_argv = [{fake}, \"--probe-directory\", {report}]\n")
        } else {
            String::new()
        };
        let m = Manifest::parse(&format!(
            "kind = \"k\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = 2\n[process]\nlaunch = [{fake}, \"--nope\"]\n\
             [resume]\nargv = [{fake}, \"--nope\", \"{{session_id}}\"]\n{fresh}"
        ))
        .unwrap();
        let mut sup = Supervisor::new(SupervisorConfig {
            node: NodeId::new("node-fresh"),
            restart: RestartPolicy {
                max_restarts: 3,
                base_delay: Duration::from_millis(10),
                max_delay: Duration::from_millis(10),
            },
            terminal: TerminalOptions::default(),
            limits: NodeLimits::default(),
        });
        sup.register(m);
        sup
    }

    /// An agent as the node restores it from the ledger after a restart, then resumed at `at`.
    fn resumed(&self, sup: &mut Supervisor, at: Instant) {
        let id = sup
            .start(
                &StartRequest {
                    kind: "k".into(),
                    checkout: self.repo.clone(),
                    worktree_root: self.root.join("worktrees"),
                    name: "a1".into(),
                    base: None,
                },
                at,
            )
            .unwrap();
        sup.stop(&id).unwrap();
        let project = agora_node::git::discover_checkout(&self.repo)
            .unwrap()
            .project;
        sup.unrestore(&id);
        sup.restore(Restore {
            id: id.clone(),
            kind: "k".into(),
            project,
            folder: self.root.join("worktrees").join("a1"),
            directory: false,
            branch: "agent/a1".into(),
            session: Some(SID.into()),
            carry_on: false,
            private_config: None,
        })
        .unwrap();
        sup.resume(&id, at).unwrap();
    }

    /// Ticks, with the clock `later` more ahead each round, until the report appears or two seconds pass.
    fn drive(&self, sup: &mut Supervisor, later: Duration) -> bool {
        let end = Instant::now() + Duration::from_secs(2);
        let mut round = 0;
        while Instant::now() < end {
            // The clock runs on by `later` every round, so a restart is as late after its own start as the first exit was.
            round += 1;
            sup.tick(Instant::now() + later * round);
            if self.report().exists() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        false
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_resume_that_fails_at_once_starts_the_session_from_nothing_when_the_manifest_says_how() {
    let e = Env::new("quick");
    let mut sup = e.supervisor(true);
    e.resumed(&mut sup, Instant::now());
    assert!(e.drive(&mut sup, Duration::ZERO), "no fresh start");
}

#[test]
fn a_kind_without_a_fresh_command_is_resumed_as_before() {
    let e = Env::new("none");
    let mut sup = e.supervisor(false);
    e.resumed(&mut sup, Instant::now());
    assert!(!e.drive(&mut sup, Duration::ZERO));
}

#[test]
fn a_resume_that_failed_after_running_a_while_is_not_taken_for_a_session_that_never_began() {
    let e = Env::new("late");
    let mut sup = e.supervisor(true);
    e.resumed(&mut sup, Instant::now());
    // Seen to end a minute after it started: an ordinary failure of a real session, which a fresh start must never overwrite.
    assert!(!e.drive(&mut sup, Duration::from_secs(60)));
}
