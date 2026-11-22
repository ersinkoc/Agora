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
    // Refused at once where the program itself is started; on macOS the started program is `sandbox-exec`, which starts and then
    // fails to run it, so the failure is that launch's end (CI job 137232241796).
    if sup.start(&rig.request("one"), Instant::now()).is_ok() {
        tick_until(&mut sup, |e| {
            e.iter()
                .any(|e| matches!(e, SupervisorEvent::Exited { .. }))
        });
    }
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

/// An adapter's `Exited` is the end of its output, not of its process (Codex review of #998): the launch's file stays while the
/// process runs on with its output closed, and goes once the process is collected.
#[test]
fn an_adapter_whose_output_ended_keeps_its_file_until_its_process_is_collected() {
    let _serial = serial();
    if !confinement() {
        return;
    }
    let rig = Rig::new("eof");
    let fake = fake_agent();
    // The fake adapter runs below the shell (not `exec`), so when it crashes the shell closes its output and lives on for a while.
    let table = format!(
        "[adapter]\ncommand = [\"/bin/sh\", \"-c\", {:?}, {:?}]",
        format!("{RECORD}; \"$0\" --adapter; exec >&-; sleep 2"),
        fake.to_str().unwrap()
    );
    let restart = RestartPolicy {
        max_restarts: 1,
        base_delay: Duration::from_secs(60),
        max_delay: Duration::from_secs(60),
    };
    let mut sup = rig.supervisor(&table, restart, vec![fake.clone()]);
    let id = sup.start(&rig.request("one"), Instant::now()).unwrap();
    let seen = rig.seen(&mut sup, "one", 1);
    let _ = sup.adapter_prompt(&id, "crash", false);
    tick_until(&mut sup, |e| {
        e.iter()
            .any(|e| matches!(e, SupervisorEvent::AdapterCrashed { .. }))
    });
    // Its output ended; its process has not.
    assert_eq!(rig.files(), [name_of(&seen[0])]);
    tick_until(&mut sup, |_| rig.files().is_empty());
    sup.stop(&id).unwrap();
}

/// A file that names its node: written by this process at a time it did not start, so by a node that is gone.
fn left_by_a_gone_node(nonce: u128) -> String {
    let me = std::process::id();
    let start = agora_platform::process_start_time(me).unwrap();
    gitconfig::launch_name(Some((me, start + 1)), nonce)
}

/// What a node start sweeps: launch files (and their staging copies) of a node that is gone, and this node's own that none of its
/// agents uses. A file with no owner in its name, and anything else in the folder, is not the node's to judge and stays.
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
    let stale = left_by_a_gone_node(0xabc);
    let unowned = gitconfig::launch_name(None, 0xdef);
    for name in [
        stale.clone(),
        format!("{stale}.new"),
        unowned.clone(),
        "neighbor".into(),
    ] {
        std::fs::write(folder.join(name), "x").unwrap();
    }
    assert_eq!(sup.sweep_launch_files(), 2);
    let mut want = vec![name_of(&seen[0]), unowned.clone(), "neighbor".to_string()];
    want.sort();
    assert_eq!(rig.files(), want);
    sup.stop(&id).unwrap();
    let mut want = vec![unowned, "neighbor".to_string()];
    want.sort();
    assert_eq!(rig.files(), want);
}

fn start_node(rig: &Rig, socket: &str) -> agora_node::daemon::Node {
    use agora_node::daemon::{Node, NodeConfig};
    let manifests = rig.dir.join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    Node::start(&NodeConfig {
        node_id: "n".into(),
        socket: rig.dir.join(socket),
        data_dir: rig.data(),
        worktree_root: rig.dir.join("worktrees"),
        manifests_dir: manifests,
        projects: vec![rig.repo.clone()],
        limits: NodeLimits::default(),
        resume_all: false,
    })
    .unwrap()
}

/// A node that starts again finds what its earlier run left (it was killed, so nothing ended its agents' launches) and removes it,
/// whatever its confinement is now.
#[test]
fn a_node_start_removes_what_an_earlier_run_left() {
    let _serial = serial();
    let rig = Rig::new("nodestart");
    let folder = gitconfig::directory(&rig.data());
    std::fs::create_dir_all(&folder).unwrap();
    let stale = left_by_a_gone_node(0x1234);
    for name in [stale.clone(), format!("{stale}.new"), "neighbor".into()] {
        std::fs::write(folder.join(name), "x").unwrap();
    }
    let node = start_node(&rig, "n.sock");
    assert_eq!(rig.files(), ["neighbor"]);
    drop(node);
}

/// Codex review of #998: a second node that starts on the same data folder (whether or not it then wins its control socket) must not
/// remove the files of the node that runs there. The running node is another process: its files name it, and it is alive.
#[test]
fn a_second_node_on_the_same_data_folder_keeps_the_running_nodes_files() {
    let _serial = serial();
    let rig = Rig::new("2nd");
    let folder = gitconfig::directory(&rig.data());
    std::fs::create_dir_all(&folder).unwrap();
    // The first node: a live process other than this one.
    let mut first = Command::new("sleep").arg("30").spawn().unwrap();
    let start = agora_platform::process_start_time(first.id()).unwrap();
    let in_use = gitconfig::launch_name(Some((first.id(), start)), 0x5);
    std::fs::write(folder.join(&in_use), "x").unwrap();
    let node = start_node(&rig, "b.sock");
    let kept = rig.files();
    drop(node);
    // Once the first node is gone, the next start removes what it left.
    first.kill().unwrap();
    first.wait().unwrap();
    let node = start_node(&rig, "c.sock");
    let after = rig.files();
    drop(node);
    assert_eq!(kept, [in_use]);
    assert_eq!(after, Vec::<String>::new());
}
