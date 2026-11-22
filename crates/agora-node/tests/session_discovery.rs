//! ADR-0060 `[session.file]`, #925: what session discovery learns about an agent belongs to one process of it. A restart or a resume
//! under the same agent id runs a new process, which waits for its own session file from its own start; an id found for the process
//! before is never set for the next. An agent that has its session keeps it (a resume continues it, also after a node restart). The
//! agent is `agora-fake-agent` (never a real agent), which stays running until its terminal closes; the session files are written by
//! the test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use agora_adapters::Manifest;
use agora_core::{AgentId, NodeId};
use agora_node::session_files::{SessionWatcher, Target};
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, Restore, StartRequest, Supervisor, SupervisorConfig, SupervisorEvent,
};
use agora_node::terminal::TerminalOptions;

/// How often the watcher looks at the files of an agent that waits.
const SCAN_EVERY: Duration = Duration::from_secs(3);
/// Longer than the slack a session file may predate its process by: a file written this long before a process started is not its.
const PAST_SLACK: Duration = Duration::from_millis(2500);

const OLD: &str = "0199aaaa-0000-4000-8000-000000000925";
const NEW: &str = "0199aaaa-0000-4000-8000-000000000926";

fn fake_agent() -> String {
    let mut p = std::env::current_exe().expect("exe");
    p.pop();
    p.pop();
    p.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    // Built once per test binary, even when present: a binary left from an older checkout may lack newer flags.
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
        let root = std::env::temp_dir().join(format!("discovery-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(root.join("home/.tool/s")).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "x").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        Self { root, repo }
    }

    /// A supervisor with kind `tool`, whose session id is read from the file it writes (as Codex's) and which resumes with it.
    fn supervisor(&self) -> Supervisor {
        let q = |v: Vec<String>| serde_json::to_string(&v).unwrap();
        let probe = |file: &str| {
            vec![
                fake_agent(),
                "--probe-directory".into(),
                self.root.join(file).to_string_lossy().into_owned(),
            ]
        };
        let mut resume = probe("resume-{pid}.json");
        resume.extend(["--resume".to_string(), "{session_id}".to_string()]);
        let m = Manifest::parse(&format!(
            "kind = \"tool\"\ndisplay_name = \"Tool\"\nversion = \"1\"\nmin_engine = 2\n[process]\nlaunch = {}\n\
             [resume]\nargv = {}\n[session.file]\nglob = \"{{home}}/.tool/s/rollout-*.jsonl\"\nid = \"payload.id\"\n\
             cwd = \"payload.cwd\"\n",
            q(probe("start-{pid}.json")),
            q(resume)
        ))
        .unwrap();
        let mut sup = Supervisor::new(SupervisorConfig {
            node: NodeId::new("node-discovery"),
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

    fn start(&self, sup: &mut Supervisor) -> AgentId {
        sup.start(
            &StartRequest {
                kind: "tool".into(),
                checkout: self.repo.clone(),
                worktree_root: self.root.join("worktrees"),
                name: "a1".into(),
                base: None,
            },
            Instant::now(),
        )
        .unwrap()
    }

    fn watcher(&self) -> SessionWatcher {
        SessionWatcher::new(Some(self.root.join("home")))
    }

    /// The session file the tool writes when it starts a session in `folder`.
    fn write_session(&self, name: &str, id: &str, folder: &Path) {
        let line = serde_json::json!({"payload": {"id": id, "cwd": folder}});
        std::fs::write(
            self.root.join("home/.tool/s").join(name),
            format!("{line}\n"),
        )
        .unwrap();
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The agent's process as session discovery sees it (the only target).
fn the_target(sup: &Supervisor) -> Target {
    let mut targets = sup.session_file_targets();
    assert_eq!(targets.len(), 1, "{targets:?}");
    targets.remove(0)
}

/// Ends the agent's process as a crash would, and ticks until the supervisor started it again.
fn crash_and_restart(sup: &mut Supervisor, id: &AgentId) {
    let pid = sup
        .agent_pids()
        .into_iter()
        .find(|(a, _)| a == id)
        .map(|(_, p)| p)
        .unwrap();
    agora_platform::terminate_process(pid).unwrap();
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let events = sup.tick(Instant::now());
        if events
            .iter()
            .any(|e| matches!(e, SupervisorEvent::Started { agent } if agent == id))
        {
            return;
        }
        assert!(Instant::now() < end, "the agent was not restarted");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn supported() -> bool {
    agora_platform::PTY_SUPPORTED
}

#[test]
fn a_restarted_agent_waits_for_its_new_processs_own_session_file() {
    if !supported() {
        return;
    }
    let e = Env::new("restart-window");
    let mut sup = e.supervisor();
    let id = e.start(&mut sup);
    let mut w = e.watcher();
    let first = the_target(&sup);
    let now = Instant::now();
    assert!(w
        .poll(now, std::slice::from_ref(&first), &HashSet::new())
        .is_empty());
    // The first process starts a session, then crashes before the next look; it is started again after the slack.
    e.write_session("rollout-old.jsonl", OLD, &first.folder);
    std::thread::sleep(PAST_SLACK);
    crash_and_restart(&mut sup, &id);
    let second = the_target(&sup);
    assert_ne!(first.launch, second.launch, "a restart runs a new process");
    assert!(
        w.poll(
            now + SCAN_EVERY,
            std::slice::from_ref(&second),
            &HashSet::new()
        )
        .is_empty(),
        "the restarted process was given the session of the process before it"
    );
    // The new process's own session is found and set.
    e.write_session("rollout-new.jsonl", NEW, &second.folder);
    let found = w.poll(
        now + SCAN_EVERY * 2,
        std::slice::from_ref(&second),
        &HashSet::new(),
    );
    assert_eq!(found, vec![(id.clone(), NEW.to_string())]);
    assert_eq!(sup.report_found_session(&id, second.launch, NEW), Ok(true));
    assert_eq!(sup.session_of(&id).as_deref(), Some(NEW));
    sup.stop(&id).unwrap();
}

#[test]
fn a_session_found_for_the_process_before_a_restart_is_not_set_for_the_next() {
    if !supported() {
        return;
    }
    let e = Env::new("restart-report");
    let mut sup = e.supervisor();
    let id = e.start(&mut sup);
    let first = the_target(&sup);
    // Found for the first process (outside the supervisor's lock); the process is restarted before it is set.
    crash_and_restart(&mut sup, &id);
    assert_eq!(sup.report_found_session(&id, first.launch, OLD), Ok(false));
    assert_eq!(sup.session_of(&id), None);
    assert!(sup.known_sessions().is_empty());
    // The new process still waits for its own.
    assert_ne!(the_target(&sup).launch, first.launch);
    sup.stop(&id).unwrap();
}

#[test]
fn a_resume_without_a_session_waits_afresh_and_one_with_its_session_keeps_it() {
    if !supported() {
        return;
    }
    let e = Env::new("resume");
    let mut sup = e.supervisor();
    let id = e.start(&mut sup);
    let first = the_target(&sup);
    // Stopped before its session was found: a resume starts it again (it has no session to continue), as a new process that waits
    // for its own file.
    sup.stop(&id).unwrap();
    assert_eq!(
        sup.report_found_session(&id, first.launch, OLD),
        Ok(false),
        "a stopped process's session was set"
    );
    sup.resume(&id, Instant::now()).unwrap();
    let second = the_target(&sup);
    assert_ne!(first.launch, second.launch);
    assert_eq!(
        sup.report_found_session(&id, first.launch, OLD),
        Ok(false),
        "the session of the process before the resume was set"
    );
    assert_eq!(sup.report_found_session(&id, second.launch, NEW), Ok(true));
    // With its session, a resume continues it: the id stays the agent's, and nothing is looked for any more.
    sup.stop(&id).unwrap();
    let events = sup.resume(&id, Instant::now()).unwrap();
    assert!(events.contains(&SupervisorEvent::Resumed {
        agent: id.clone(),
        native: true
    }));
    assert_eq!(sup.session_of(&id).as_deref(), Some(NEW));
    assert!(sup.session_file_targets().is_empty());
    sup.stop(&id).unwrap();
}

#[test]
fn an_agent_restored_after_a_node_restart_keeps_its_own_session() {
    if !supported() {
        return;
    }
    let e = Env::new("node-restart");
    let mut sup = e.supervisor();
    let id = e.start(&mut sup);
    let folder = the_target(&sup).folder;
    sup.stop(&id).unwrap();
    let project = agora_node::git::discover_checkout(&e.repo).unwrap().project;
    // As the next node puts it back from the ledger, with the session that was found for it before.
    sup.unrestore(&id);
    sup.restore(Restore {
        id: id.clone(),
        kind: "tool".into(),
        project,
        folder,
        directory: false,
        branch: "agent/a1".into(),
        session: Some(OLD.into()),
        carry_on: false,
        private_config: None,
    })
    .unwrap();
    sup.resume(&id, Instant::now()).unwrap();
    assert_eq!(sup.session_of(&id).as_deref(), Some(OLD));
    assert!(sup.session_file_targets().is_empty());
    // Nothing found by discovery replaces it, not even for the process that runs now.
    let launch = sup.terminal(&id).unwrap().launch();
    assert_eq!(sup.report_found_session(&id, launch, NEW), Ok(false));
    assert_eq!(sup.session_of(&id).as_deref(), Some(OLD));
    sup.stop(&id).unwrap();
}
