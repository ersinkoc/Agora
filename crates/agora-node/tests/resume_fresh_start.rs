//! ADR-0060, `[resume] fresh_argv`: an agent whose tool wrote no session until it was spoken to (Claude Code) has nothing to resume
//! after a node restart. A resume that fails within moments starts the session from nothing under the same id, for a kind whose
//! manifest says how; otherwise nothing changes. The "tool" is a shell one-liner (never a real agent) that appends `R` (resume
//! command) or `F` (fresh command) with the session id to a log and exits with a chosen code, so each test asserts what was launched.
#![cfg(unix)]
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

    fn log(&self) -> PathBuf {
        self.root.join("launches.log")
    }

    /// What was launched so far, one `R <id>` or `F <id>` per line.
    fn launches(&self) -> Vec<String> {
        std::fs::read_to_string(self.log())
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// A supervisor with a kind `k`: its resume command logs `R` and exits with `resume_exit`; its `fresh_argv` (when `fresh_exit` is
    /// given) logs `F` and exits with that code.
    fn supervisor(&self, resume_exit: i32, fresh_exit: Option<i32>) -> Supervisor {
        let log = self.log().to_string_lossy().into_owned();
        let argv = |tag: &str, code: i32| {
            serde_json::to_string(&[
                "/bin/sh",
                "-c",
                &format!("echo \"{tag} $1\" >> '{log}'; exit {code}"),
                "sh",
                "{session_id}",
            ])
            .unwrap()
        };
        let fresh = fresh_exit
            .map(|c| format!("fresh_argv = {}\n", argv("F", c)))
            .unwrap_or_default();
        let launch = serde_json::to_string(&["/bin/sh", "-c", "exit 1"]).unwrap();
        let m = Manifest::parse(&format!(
            "kind = \"k\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = 2\n[process]\nlaunch = {launch}\n\
             [resume]\nargv = {}\n{fresh}",
            argv("R", resume_exit)
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

    /// An agent as the node restores it from the ledger after a restart, then resumed.
    fn resumed(&self, sup: &mut Supervisor) {
        let at = Instant::now();
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
        // The start above ran `exit 1` and the stop ended it; only the resume logs. Clear the log all the same.
        let _ = std::fs::remove_file(self.log());
        sup.resume(&id, at).unwrap();
    }

    /// Ticks for a while, the clock `later` more ahead each round (so a restart is as late after its own start as the first exit
    /// was), and returns what was launched. Long enough for every restart to be due and every process to end.
    fn run(&self, sup: &mut Supervisor, later: Duration) -> Vec<String> {
        let end = Instant::now() + Duration::from_millis(1500);
        let mut round = 0;
        while Instant::now() < end {
            round += 1;
            sup.tick(Instant::now() + later * round);
            std::thread::sleep(Duration::from_millis(15));
        }
        self.launches()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn lines(tags: &str) -> Vec<String> {
    tags.chars().map(|t| format!("{t} {SID}")).collect()
}

#[test]
fn a_resume_that_fails_at_once_starts_the_session_from_nothing_under_the_same_id() {
    let e = Env::new("quick");
    let mut sup = e.supervisor(3, Some(0));
    e.resumed(&mut sup);
    // Resume fails, the fresh start (which works) runs once under the same id; its success ends the agent there.
    assert_eq!(e.run(&mut sup, Duration::ZERO), lines("RF"));
}

#[test]
fn a_fresh_start_that_fails_too_is_followed_by_the_resume_again_and_restarts_stay_bounded() {
    let e = Env::new("both");
    let mut sup = e.supervisor(3, Some(3));
    e.resumed(&mut sup);
    // The resume, then at most `max_restarts` (3) restarts: fresh, resume, fresh. A fresh start's own quick failure is not taken for
    // a failed resume, so it never repeats itself.
    assert_eq!(e.run(&mut sup, Duration::ZERO), lines("RFRF"));
}

#[test]
fn a_kind_without_a_fresh_command_is_resumed_as_before() {
    let e = Env::new("none");
    let mut sup = e.supervisor(3, None);
    e.resumed(&mut sup);
    assert_eq!(e.run(&mut sup, Duration::ZERO), lines("RRRR"));
}

#[test]
fn a_resume_that_failed_after_the_window_is_resumed_again_never_started_fresh() {
    let e = Env::new("late");
    let mut sup = e.supervisor(3, Some(0));
    e.resumed(&mut sup);
    // Seen to end a minute after it started: an ordinary failure of a real session, which a fresh start must never overwrite.
    assert_eq!(e.run(&mut sup, Duration::from_secs(60)), lines("RRRR"));
}

#[test]
fn a_resume_that_ends_well_within_the_window_never_starts_a_fresh_one() {
    let e = Env::new("zero");
    let mut sup = e.supervisor(0, Some(0));
    e.resumed(&mut sup);
    assert_eq!(e.run(&mut sup, Duration::ZERO), lines("R"));
}
