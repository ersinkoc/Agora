//! #933 (ADR-0098): a launch's generated Git configuration lives exactly as long as the launch. The node's `git-config/` folder holds
//! only the files of agents that run: a file goes when its agent ends, is stopped or crashes, and when its start fails after the file
//! was written; a restart gets a fresh file; a node start removes what an earlier run left behind and spares what a live agent uses.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use agora_adapters::Manifest;
use agora_core::NodeId;
use agora_node::gitconfig;
use agora_node::sandbox::{Level, SandboxPolicy};
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, StartRequest, Supervisor, SupervisorConfig, SupervisorEvent,
};
use agora_node::terminal::TerminalOptions;

fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A home with a small global configuration, so nothing of the real person's is read.
fn home() -> &'static PathBuf {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        // Under the target folder, never under `/tmp`: the sandbox opens `/tmp`, and a folder an agent may write gets no file.
        let h = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("gccleanup-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&h);
        std::fs::create_dir_all(&h).unwrap();
        std::fs::write(h.join(".gitconfig"), "[user]\n\tname = Ada Example\n").unwrap();
        std::env::set_var("HOME", &h);
        for v in [
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_SYSTEM",
            "GIT_CONFIG_COUNT",
            "XDG_CONFIG_HOME",
        ] {
            std::env::remove_var(v);
        }
        std::env::set_var("GIT_CEILING_DIRECTORIES", env!("CARGO_TARGET_TMPDIR"));
        h
    })
}

fn git(dir: &Path, args: &[&str]) {
    assert!(Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@e")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@e")
        .status()
        .unwrap()
        .success());
}

/// A test's own folder (removed when it is dropped) with a one-commit repository in it.
struct Rig {
    dir: PathBuf,
    repo: PathBuf,
}

impl Rig {
    fn new(name: &str) -> Self {
        home();
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("gccleanup-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "x").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        std::fs::create_dir_all(dir.join("data")).unwrap();
        Self { dir, repo }
    }

    fn data(&self) -> PathBuf {
        self.dir.join("data")
    }

    /// The files in the node's generated Git folder, by name (none when the folder was never made).
    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(gitconfig::directory(&self.data()))
            .map(|d| {
                d.filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// A confining supervisor with one kind, `shy`, from the given manifest table.
    fn supervisor(&self, table: &str, restart: RestartPolicy, read: Vec<PathBuf>) -> Supervisor {
        let manifest = Manifest::parse(&format!(
            "kind = \"shy\"\ndisplay_name = \"Shy\"\nversion = \"1\"\nmin_engine = 1\n{table}\n"
        ))
        .unwrap();
        let mut sup = Supervisor::new(SupervisorConfig {
            node: NodeId::new("node-1"),
            restart,
            terminal: TerminalOptions::default(),
            limits: NodeLimits::default(),
        });
        sup.register(manifest);
        sup.set_sandbox(SandboxPolicy {
            level: Level::Worktree,
            read,
            protect: vec![self.data()],
            data_dir: Some(self.data()),
            ..Default::default()
        });
        sup
    }

    fn request(&self, name: &str) -> StartRequest {
        StartRequest {
            kind: "shy".into(),
            checkout: self.repo.clone(),
            worktree_root: self.dir.join("worktrees"),
            name: name.into(),
            base: None,
        }
    }

    /// What the agent named `name` wrote to `gcg` in its worktree (one `$GIT_CONFIG_GLOBAL` per line), waiting for `lines` of it.
    fn seen(&self, sup: &mut Supervisor, name: &str, lines: usize) -> Vec<String> {
        let file = self.dir.join("worktrees").join(name).join("gcg");
        let end = Instant::now() + Duration::from_secs(15);
        loop {
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            let got: Vec<String> = text.lines().map(str::to_string).collect();
            if got.len() >= lines || Instant::now() >= end {
                assert_eq!(got.len(), lines, "{text:?}");
                return got;
            }
            sup.tick(Instant::now());
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn name_of(path: &str) -> String {
    Path::new(path)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn terminal(script: &str) -> String {
    format!("[process]\nlaunch = [\"/bin/sh\", \"-c\", {script:?}]")
}

/// Ticks until `want` holds of what the tick reported, for at most 15 s.
fn tick_until(sup: &mut Supervisor, mut want: impl FnMut(&[SupervisorEvent]) -> bool) {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        let events = sup.tick(Instant::now());
        if want(&events) {
            return;
        }
        assert!(Instant::now() < end, "timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn confinement() -> bool {
    let ok = agora_platform::sandbox_supported();
    if !ok {
        eprintln!("skipped: no confinement on this machine");
    }
    ok
}

const RECORD: &str = "printf '%s\\n' \"$GIT_CONFIG_GLOBAL\" >> gcg";

#[test]
fn a_running_agent_keeps_its_file_and_a_stop_removes_it() {
    let _serial = serial();
    if !confinement() {
        return;
    }
    let rig = Rig::new("stop");
    let mut sup = rig.supervisor(
        &terminal(&format!("{RECORD}; sleep 30")),
        RestartPolicy::default(),
        vec![],
    );
    let one = sup.start(&rig.request("one"), Instant::now()).unwrap();
    let two = sup.start(&rig.request("two"), Instant::now()).unwrap();
    let a = rig.seen(&mut sup, "one", 1);
    let b = rig.seen(&mut sup, "two", 1);
    assert_ne!(a, b);
    // While they run, each keeps its own file and nothing else is there.
    let mut both = vec![name_of(&a[0]), name_of(&b[0])];
    both.sort();
    assert_eq!(rig.files(), both);
    sup.stop(&one).unwrap();
    assert_eq!(rig.files(), [name_of(&b[0])]);
    sup.stop(&two).unwrap();
    assert_eq!(rig.files(), Vec::<String>::new());
}

#[test]
fn an_agent_that_ends_loses_its_file() {
    let _serial = serial();
    if !confinement() {
        return;
    }
    let rig = Rig::new("end");
    let mut sup = rig.supervisor(
        &terminal(&format!("{RECORD}; exit 0")),
        RestartPolicy::default(),
        vec![],
    );
    sup.start(&rig.request("one"), Instant::now()).unwrap();
    tick_until(&mut sup, |e| {
        e.iter()
            .any(|e| matches!(e, SupervisorEvent::Exited { .. }))
    });
    rig.seen(&mut sup, "one", 1);
    assert_eq!(rig.files(), Vec::<String>::new());
}

#[test]
fn a_crash_removes_the_file_and_the_restart_gets_a_fresh_one() {
    let _serial = serial();
    if !confinement() {
        return;
    }
    let rig = Rig::new("restart");
    // The first run crashes, the second keeps running.
    let script = format!("{RECORD}; if [ -f again ]; then sleep 30; fi; touch again; exit 3");
    let mut sup = rig.supervisor(
        &terminal(&script),
        RestartPolicy {
            max_restarts: 3,
            base_delay: Duration::from_millis(300),
            max_delay: Duration::from_millis(300),
        },
        vec![],
    );
    let id = sup.start(&rig.request("one"), Instant::now()).unwrap();
    tick_until(&mut sup, |e| {
        e.iter()
            .any(|e| matches!(e, SupervisorEvent::Restarting { .. }))
    });
    // Between the crash and the restart nothing runs, so nothing is kept.
    assert_eq!(rig.files(), Vec::<String>::new());
    tick_until(&mut sup, |e| {
        e.iter()
            .any(|e| matches!(e, SupervisorEvent::Started { .. }))
    });
    let seen = rig.seen(&mut sup, "one", 2);
    assert_ne!(seen[0], seen[1], "a restart gets a file of its own");
    assert_eq!(rig.files(), [name_of(&seen[1])]);
    sup.stop(&id).unwrap();
    assert_eq!(rig.files(), Vec::<String>::new());
}

#[test]
fn a_start_that_fails_after_the_file_was_written_leaves_none() {
    let _serial = serial();
    if !confinement() {
        return;
    }
    let rig = Rig::new("failed");
    // A terminal agent whose program is not there.
    let missing = rig.dir.join("missing-program");
    let mut sup = rig.supervisor(
        &format!("[process]\nlaunch = [{:?}]", missing.to_str().unwrap()),
        RestartPolicy::default(),
        vec![],
    );
    assert!(sup.start(&rig.request("one"), Instant::now()).is_err());
    assert_eq!(rig.files(), Vec::<String>::new());
    // An adapter that ends before it answers.
    let mut sup = rig.supervisor(
        "[adapter]\ncommand = [\"/bin/sh\", \"-c\", \"exit 1\"]",
        RestartPolicy::default(),
        vec![],
    );
    assert!(sup.start(&rig.request("two"), Instant::now()).is_err());
    assert_eq!(rig.files(), Vec::<String>::new());
}

fn fake_agent() -> PathBuf {
    let path = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("agora-fake-agent");
    assert!(Command::new(env!("CARGO"))
        .args(["build", "--locked", "-q", "-p", "agora-fake-agent"])
        .status()
        .unwrap()
        .success());
    path
}

#[test]
fn a_structured_agent_keeps_its_file_while_it_runs_and_a_stop_removes_it() {
    let _serial = serial();
    if !confinement() {
        return;
    }
    let rig = Rig::new("adapter");
    let fake = fake_agent();
    let table = format!(
        "[adapter]\ncommand = [\"/bin/sh\", \"-c\", {:?}, {:?}]",
        format!("{RECORD}; exec \"$0\" --adapter"),
        fake.to_str().unwrap()
    );
    let mut sup = rig.supervisor(&table, RestartPolicy::default(), vec![fake.clone()]);
    let id = sup.start(&rig.request("one"), Instant::now()).unwrap();
    let seen = rig.seen(&mut sup, "one", 1);
    assert_eq!(rig.files(), [name_of(&seen[0])]);
    sup.stop(&id).unwrap();
    assert_eq!(rig.files(), Vec::<String>::new());
}

/// What a node start sweeps: the node's own per-launch names (and their staging files) that no agent of this node uses. Anything else
/// in the folder is not the node's to judge and stays.
#[test]
fn a_sweep_removes_left_files_and_spares_those_of_running_agents() {
    let _serial = serial();
    if !confinement() {
        return;
    }
    let rig = Rig::new("sweep");
    let mut sup = rig.supervisor(
        &terminal(&format!("{RECORD}; sleep 30")),
        RestartPolicy::default(),
        vec![],
    );
    let id = sup.start(&rig.request("one"), Instant::now()).unwrap();
    let seen = rig.seen(&mut sup, "one", 1);
    let folder = gitconfig::directory(&rig.data());
    let stale = format!("agent-{:032x}", 0xabcu128);
    for name in [stale.clone(), format!("{stale}.new"), "neighbor".into()] {
        std::fs::write(folder.join(name), "x").unwrap();
    }
    sup.sweep_launch_files();
    let mut want = vec![name_of(&seen[0]), "neighbor".to_string()];
    want.sort();
    assert_eq!(rig.files(), want);
    sup.stop(&id).unwrap();
    assert_eq!(rig.files(), ["neighbor"]);
}

/// A node that starts again finds what its earlier run left (it was killed, so nothing ended its agents' launches) and removes it,
/// whatever its confinement is now.
#[test]
fn a_node_start_removes_what_an_earlier_run_left() {
    use agora_node::daemon::{Node, NodeConfig};

    let _serial = serial();
    let rig = Rig::new("nodestart");
    let folder = gitconfig::directory(&rig.data());
    std::fs::create_dir_all(&folder).unwrap();
    let stale = format!("agent-{:032x}", 0x1234u128);
    for name in [stale.clone(), format!("{stale}.new"), "neighbor".into()] {
        std::fs::write(folder.join(name), "x").unwrap();
    }
    let manifests = rig.dir.join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    let node = Node::start(&NodeConfig {
        node_id: "n".into(),
        socket: rig.dir.join("n.sock"),
        data_dir: rig.data(),
        worktree_root: rig.dir.join("worktrees"),
        manifests_dir: manifests,
        projects: vec![rig.repo.clone()],
        limits: NodeLimits::default(),
        resume_all: false,
    })
    .unwrap();
    assert_eq!(rig.files(), ["neighbor"]);
    drop(node);
}
