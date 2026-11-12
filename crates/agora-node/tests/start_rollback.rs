//! Failed launches roll back only the worktree and unchanged branch they created.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use agora_adapters::Manifest;
use agora_core::NodeId;
use agora_node::terminal::TerminalOptions;
use agora_node::{
    git,
    supervisor::{NodeLimits, RestartPolicy, StartRequest, Supervisor, SupervisorConfig},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU32, Ordering},
    time::Instant,
};

struct Repo {
    root: PathBuf,
    checkout: PathBuf,
}
impl Repo {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "agora-start-rollback-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let checkout = root.join("repo");
        fs::create_dir_all(&checkout).unwrap();
        raw(&checkout, &["init", "-q", "-b", "main"]);
        raw(&checkout, &["config", "core.autocrlf", "false"]);
        fs::write(checkout.join("a.txt"), "initial\n").unwrap();
        raw(&checkout, &["add", "a.txt"]);
        raw(&checkout, &["commit", "-q", "-m", "initial"]);
        Self { root, checkout }
    }
    fn request(&self) -> StartRequest {
        StartRequest {
            kind: "fixture".into(),
            checkout: self.checkout.clone(),
            worktree_root: self.root.join("worktrees"),
            name: "attempt".into(),
            base: None,
        }
    }
    fn supervisor(&self, adapter: bool) -> Supervisor {
        let missing = self.root.join(format!(
            "missing-agora-fake-agent{}",
            std::env::consts::EXE_SUFFIX
        ));
        let table = if adapter {
            "[adapter]\ncommand"
        } else {
            "[process]\nlaunch"
        };
        let manifest = Manifest::parse(&format!("kind = \"fixture\"\ndisplay_name = \"Fixture\"\nversion = \"1\"\nmin_engine = 1\n{table} = [{}]\n", serde_json::to_string(&missing.to_string_lossy()).unwrap())).unwrap();
        let mut supervisor = Supervisor::new(SupervisorConfig {
            node: NodeId::new("fixture"),
            restart: RestartPolicy::default(),
            terminal: TerminalOptions::default(),
            limits: NodeLimits::default(),
        });
        supervisor.register(manifest);
        supervisor
    }
    fn branch(&self) -> String {
        raw(
            &self.checkout,
            &["rev-parse", "--verify", "refs/heads/agent/attempt"],
        )
        .trim()
        .into()
    }
    fn no_branch(&self) {
        assert_eq!(
            run(
                &self.checkout,
                &[
                    "show-ref",
                    "--verify",
                    "--quiet",
                    "refs/heads/agent/attempt"
                ]
            )
            .status
            .code(),
            Some(1)
        );
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "commit.gpgsign=false",
            "-C",
        ])
        .arg(dir)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_CONFIG_COUNT")
        .env("GIT_AUTHOR_NAME", "fixture")
        .env("GIT_COMMITTER_NAME", "fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap()
}
fn raw(dir: &Path, args: &[&str]) -> String {
    let out = run(dir, args);
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn failed_terminal_start_removes_its_unchanged_branch_and_can_retry() {
    let repo = Repo::new();
    let req = repo.request();
    let mut supervisor = repo.supervisor(false);
    for _ in 0..2 {
        let error = supervisor.start(&req, Instant::now()).unwrap_err();
        assert!(
            matches!(error, agora_node::supervisor::SupervisorError::Spawn(_)),
            "{error:?}"
        );
        assert!(!req.worktree_root.join(&req.name).exists());
        repo.no_branch();
    }
    assert_eq!(
        fs::read_to_string(repo.checkout.join("a.txt")).unwrap(),
        "initial\n"
    );
}

#[test]
fn failed_selected_worktree_start_removes_its_unchanged_branch() {
    let repo = Repo::new();
    let req = repo.request();
    let project = git::discover_checkout(&repo.checkout).unwrap().project;
    assert!(repo
        .supervisor(false)
        .start_selected(&req, project, &repo.checkout, true, Instant::now())
        .is_err());
    assert!(!req.worktree_root.join(&req.name).exists());
    repo.no_branch();
}

#[test]
fn failed_adapter_start_removes_its_worktree_and_unchanged_branch() {
    let repo = Repo::new();
    let req = repo.request();
    assert!(repo.supervisor(true).start(&req, Instant::now()).is_err());
    assert!(!req.worktree_root.join(&req.name).exists());
    repo.no_branch();
}

#[test]
fn failed_injection_cleans_its_branch_and_aborts_the_endpoint_once() {
    use std::sync::{atomic::AtomicUsize, Arc};
    for separate in [false, true] {
        let repo = Repo::new();
        let req = repo.request();
        let mut supervisor = repo.supervisor(false);
        supervisor.register(Manifest::parse("kind = \"fixture\"\ndisplay_name = \"Fixture\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"missing-agora-fake-agent\"]\n[mcp]\nmethod = \"config_file\"\npath = \"../outside.json\"\n").unwrap());
        supervisor.set_launch_hook(Box::new(|launch| {
            Some(agora_adapters::inject::Endpoint {
                command: launch.worktree.join("agora-fake-agent"),
                socket: launch.worktree.join("mesh.sock"),
            })
        }));
        let aborted = Arc::new(AtomicUsize::new(0));
        let observed = aborted.clone();
        supervisor.set_abort_hook(Box::new(move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
        }));
        let result = if separate {
            let project = git::discover_checkout(&repo.checkout).unwrap().project;
            supervisor.start_selected(&req, project, &repo.checkout, true, Instant::now())
        } else {
            supervisor.start(&req, Instant::now())
        };
        let error = result.unwrap_err();
        assert!(
            matches!(&error, agora_node::supervisor::SupervisorError::Spawn(message) if message.starts_with("mcp injection:")),
            "{error:?}"
        );
        assert_eq!(aborted.load(Ordering::SeqCst), 1);
        assert!(!req.worktree_root.join(&req.name).exists());
        assert!(!req.worktree_root.join("outside.json").exists());
        repo.no_branch();
    }
}

#[test]
fn an_explicit_base_is_rolled_back_against_that_commit_not_the_current_head() {
    let repo = Repo::new();
    let initial = raw(&repo.checkout, &["rev-parse", "HEAD"]);
    fs::write(repo.checkout.join("later.txt"), "later commit\n").unwrap();
    raw(&repo.checkout, &["add", "later.txt"]);
    raw(&repo.checkout, &["commit", "-q", "-m", "later"]);
    let mut req = repo.request();
    req.base = Some(initial.trim().into());
    assert!(repo.supervisor(false).start(&req, Instant::now()).is_err());
    repo.no_branch();
    assert_eq!(
        fs::read_to_string(repo.checkout.join("later.txt")).unwrap(),
        "later commit\n"
    );
}

#[test]
fn a_commit_made_during_start_is_preserved_on_its_branch() {
    let repo = Repo::new();
    let req = repo.request();
    let mut supervisor = repo.supervisor(false);
    supervisor.set_launch_hook(Box::new(|launch| {
        fs::write(launch.worktree.join("saved.txt"), "preserve this commit\n").unwrap();
        raw(&launch.worktree, &["add", "saved.txt"]);
        raw(&launch.worktree, &["commit", "-q", "-m", "concurrent work"]);
        None
    }));
    let initial = raw(&repo.checkout, &["rev-parse", "HEAD"]);
    assert!(supervisor.start(&req, Instant::now()).is_err());
    assert_ne!(repo.branch(), initial.trim());
    assert_eq!(
        raw(&repo.checkout, &["show", "agent/attempt:saved.txt"]),
        "preserve this commit\n"
    );
    assert!(!req.worktree_root.join(&req.name).exists());
}

#[test]
fn a_branch_checked_out_elsewhere_is_preserved() {
    let repo = Repo::new();
    let req = repo.request();
    let target = req.worktree_root.join(&req.name);
    let checkout = repo.checkout.clone();
    let elsewhere = repo.root.join("elsewhere");
    let other = elsewhere.clone();
    let mut supervisor = repo.supervisor(false);
    supervisor.set_abort_hook(Box::new(move |_| {
        raw(&target, &["checkout", "--detach", "-q"]);
        raw(
            &checkout,
            &["worktree", "add", other.to_str().unwrap(), "agent/attempt"],
        );
    }));
    assert!(supervisor.start(&req, Instant::now()).is_err());
    assert!(!repo.branch().is_empty());
    assert_eq!(
        fs::read_to_string(elsewhere.join("a.txt")).unwrap(),
        "initial\n"
    );
    git::remove_worktree(&repo.checkout, &elsewhere, true).unwrap();
}

#[test]
fn refused_worktree_removal_keeps_the_branch_and_worktree() {
    let repo = Repo::new();
    let req = repo.request();
    let target = req.worktree_root.join(&req.name);
    let checkout = repo.checkout.clone();
    let locked = target.clone();
    let mut supervisor = repo.supervisor(false);
    supervisor.set_abort_hook(Box::new(move |_| {
        raw(&checkout, &["worktree", "lock", locked.to_str().unwrap()]);
    }));
    assert!(supervisor.start(&req, Instant::now()).is_err());
    assert!(target.exists());
    assert!(!repo.branch().is_empty());
    raw(
        &repo.checkout,
        &["worktree", "unlock", target.to_str().unwrap()],
    );
    git::remove_worktree(&repo.checkout, &target, true).unwrap();
    assert!(
        !repo.branch().is_empty(),
        "ordinary removal preserves branches"
    );
}

#[test]
fn an_existing_branch_or_selected_directory_is_never_rolled_back() {
    let repo = Repo::new();
    let req = repo.request();
    raw(&repo.checkout, &["branch", "agent/attempt"]);
    let initial = repo.branch();
    assert!(repo.supervisor(false).start(&req, Instant::now()).is_err());
    assert_eq!(repo.branch(), initial);
    fs::write(repo.checkout.join("uncommitted.txt"), "keep this\n").unwrap();
    let project = git::discover_checkout(&repo.checkout).unwrap().project;
    assert!(repo
        .supervisor(false)
        .start_in_directory(&req, project, Instant::now())
        .is_err());
    assert_eq!(repo.branch(), initial);
    assert_eq!(
        fs::read_to_string(repo.checkout.join("uncommitted.txt")).unwrap(),
        "keep this\n"
    );
}

#[test]
fn a_rewritten_symbolic_branch_does_not_delete_the_users_main_branch() {
    let repo = Repo::new();
    let req = repo.request();
    let checkout = repo.checkout.clone();
    let initial = raw(&repo.checkout, &["rev-parse", "HEAD"]);
    let mut supervisor = repo.supervisor(false);
    supervisor.set_abort_hook(Box::new(move |_| {
        raw(
            &checkout,
            &[
                "symbolic-ref",
                "refs/heads/agent/attempt",
                "refs/heads/main",
            ],
        );
    }));
    assert!(supervisor.start(&req, Instant::now()).is_err());
    assert_eq!(
        raw(&repo.checkout, &["rev-parse", "refs/heads/main"]),
        initial
    );
    assert_eq!(
        raw(
            &repo.checkout,
            &["symbolic-ref", "refs/heads/agent/attempt"]
        )
        .trim(),
        "refs/heads/main"
    );
}
