//! ADR-0060: a real node restart. Agents are started with `agora-fake-agent` (never a real agent), the node object is dropped (its
//! agents die with it, as when a machine stops), a new node starts on the same data folder, the agents are interrupted, and Resume
//! brings them back under the same id with the same session id.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use agora_node::{
    control::Client,
    daemon::{Node, NodeConfig},
    supervisor::NodeLimits,
};
use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

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
        // Even when present: a binary left from an older checkout may lack newer behaviour.
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

const SENTENCE: &str =
    "The session was cut off by a restart. Check the state first, then carry on.";

fn supported() -> bool {
    agora_platform::PTY_SUPPORTED && agora_platform::LOCAL_SOCKETS_SUPPORTED
}

/// How long one environment may live before its test is called hung (#1011). The suite takes under a minute; a hang used to run until
/// the CI step's 25-minute limit, with the scratch agents still alive, because a call to the node (or a node's own stop) can block
/// where no wait below is checking the time.
const HUNG_AFTER: Duration = Duration::from_secs(300);

/// Watches one environment from another thread: if it is still there after [`HUNG_AFTER`], it says what the test was doing and what
/// the folder holds, ends this process's scratch agents (none of it needs a lock a stuck thread may hold), and ends the test run with
/// a failure. Disarmed when dropped.
struct Watchdog {
    done: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Watchdog {
    fn arm(root: PathBuf, doing: Arc<Mutex<String>>) -> Self {
        let done = Arc::new((Mutex::new(false), Condvar::new()));
        let seen = Arc::clone(&done);
        let thread = std::thread::spawn(move || {
            let end = Instant::now() + HUNG_AFTER;
            let (lock, wake) = &*seen;
            let mut finished = lock.lock().unwrap_or_else(|e| e.into_inner());
            while !*finished {
                let Some(left) = end.checked_duration_since(Instant::now()) else {
                    break;
                };
                finished = wake
                    .wait_timeout(finished, left)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            if *finished {
                return;
            }
            let doing = doing.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let said = format!(
                "{} hung for {HUNG_AFTER:?} while {doing}\n{}ended the scratch agents: {:?}\n",
                root.display(),
                folder_state(&root),
                end_scratch_agents(None)
            );
            // Straight to the standard error: the test harness holds back what `eprintln!` says until a test ends, and this one will not.
            let _ = std::io::Write::write_all(&mut std::io::stderr(), said.as_bytes());
            std::process::exit(101);
        });
        Self {
            done,
            thread: Some(thread),
        }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        let (lock, wake) = &*self.done;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
        wake.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Ends the fake agents this test process started (`only`: just these pids), without asking any node: a node whose thread is stuck may
/// hold the lock that knows them. Each process is identified first; only an identity that names a fake agent and is still this
/// process's child is ended, and only while it is still that process (a pid can be given again). One that cannot be identified is
/// left alone and said so (on Windows its node's job ends it with the node). Says which pids it ended.
fn end_scratch_agents(only: Option<&[u32]>) -> Vec<u32> {
    let mut ended = Vec::new();
    let is_child = |pid: u32| {
        agora_platform::process_children(std::process::id()).is_ok_and(|c| c.contains(&pid))
    };
    for pid in agora_platform::process_children(std::process::id()).unwrap_or_default() {
        if only.is_some_and(|o| !o.contains(&pid)) {
            continue;
        }
        match end_if_ours(
            pid,
            agora_platform::process_identity,
            is_child,
            agora_platform::kill_process_if_same,
        ) {
            Ending::Ended => ended.push(pid),
            Ending::NotOurs | Ending::Gone => {}
            // Straight to the standard error: this may run on the watchdog's thread just before the process exits.
            Ending::Skipped(why) => {
                let _ = std::io::Write::write_all(
                    &mut std::io::stderr(),
                    format!("left child {pid} alone: it cannot be identified ({why})\n").as_bytes(),
                );
            }
        }
    }
    ended
}

/// What [`end_if_ours`] did with one pid.
#[derive(Debug, PartialEq, Eq)]
enum Ending {
    Ended,
    /// The identity read names no fake agent, or that pid is no longer this process's child: nothing was signalled.
    NotOurs,
    /// It was no longer the process that was identified (or had ended): nothing was signalled.
    Gone,
    /// It could not be identified, so nothing was signalled.
    Skipped(String),
}

/// Ends `pid` only through the identity read first (`identify`): that identity must name a fake agent, the pid must still be this
/// process's child after it was read (`is_child`), and `kill_if_same` signals only the process that identity names. A pid that cannot
/// be identified is never signalled by its bare number, which may by now name another process (Codex review of #1030).
fn end_if_ours(
    pid: u32,
    identify: impl Fn(u32) -> std::io::Result<agora_platform::ProcessIdentity>,
    is_child: impl Fn(u32) -> bool,
    kill_if_same: impl Fn(&agora_platform::ProcessIdentity) -> std::io::Result<bool>,
) -> Ending {
    let id = match identify(pid) {
        Ok(id) => id,
        Err(e) => return Ending::Skipped(e.to_string()),
    };
    if !id.name.contains("agora-fake") || !is_child(id.pid) {
        return Ending::NotOurs;
    }
    match kill_if_same(&id) {
        Ok(true) => Ending::Ended,
        Ok(false) => Ending::Gone,
        Err(e) => Ending::Skipped(e.to_string()),
    }
}

#[test]
fn the_watchdog_never_signals_a_pid_it_cannot_identify_or_that_is_not_its_fake_agent() {
    let signalled = std::cell::Cell::new(false);
    let kill = |_: &agora_platform::ProcessIdentity| {
        signalled.set(true);
        Ok(true)
    };
    let child = |_: u32| true;
    // Unsupported (Windows) or a passing error (Unix): nothing is signalled, and the reason is kept to be said.
    for err in [
        std::io::ErrorKind::Unsupported,
        std::io::ErrorKind::NotFound,
        std::io::ErrorKind::PermissionDenied,
    ] {
        let ending = end_if_ours(4242, |_| Err(std::io::Error::from(err)), child, kill);
        assert!(matches!(ending, Ending::Skipped(_)), "{err:?}: {ending:?}");
        assert!(
            !signalled.get(),
            "{err:?}: a pid that could not be identified was signalled"
        );
    }
    let fake = agora_platform::ProcessIdentity {
        pid: 4242,
        group: 4242,
        started: 1,
        boot: String::new(),
        name: "agora-fake-agen".into(),
    };
    // The identity read names another program (the pid was given to a replacement before it was read): not signalled.
    let other = agora_platform::ProcessIdentity {
        name: "sshd".into(),
        ..fake.clone()
    };
    assert_eq!(
        end_if_ours(4242, |_| Ok(other.clone()), child, kill),
        Ending::NotOurs
    );
    // A fake agent that is no longer this process's child once identified: not signalled.
    assert_eq!(
        end_if_ours(4242, |_| Ok(fake.clone()), |_| false, kill),
        Ending::NotOurs
    );
    assert!(!signalled.get());
    // A child fake agent goes through the identity-checked kill only.
    assert_eq!(
        end_if_ours(4242, |_| Ok(fake.clone()), child, kill),
        Ending::Ended
    );
    assert!(signalled.get());
}

/// What the folder of an environment says, read without the node: the ledger file, the files, and each probe report's arguments.
fn folder_state(root: &Path) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let ledger_file = root
        .join("data")
        .join("sessions")
        .join(agora_node::ledger::LEDGER_FILE);
    let _ = writeln!(
        out,
        "ledger file {}: {:?}",
        ledger_file.display(),
        std::fs::read_to_string(&ledger_file)
    );
    let mut files: Vec<String> = std::fs::read_dir(root)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let _ = writeln!(out, "files in {}: {files:?}", root.display());
    // A report is named by the pid that wrote it: one name for two starts (a pid given again) or a start that never wrote one shows here.
    for f in files.iter().filter(|f| f.ends_with(".json")) {
        let _ = writeln!(
            out,
            "  {f}: {:?}",
            std::fs::read_to_string(root.join(f)).unwrap_or_default()
        );
    }
    out
}

struct Env {
    root: PathBuf,
    node: Option<Node>,
    /// `agora node run --resume auto` (ADR-0062 item 4).
    resume_all: bool,
    /// What the test is doing now, for [`Watchdog`].
    doing: Arc<Mutex<String>>,
    watchdog: Option<Watchdog>,
}

impl Env {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("resume-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("manifests")).unwrap();
        std::fs::create_dir_all(root.join("work/a")).unwrap();
        let q = |v: Vec<String>| serde_json::to_string(&v).unwrap();
        let probe = |file: &str| {
            vec![
                fake(),
                "--probe-directory".into(),
                root.join(file).to_string_lossy().into_owned(),
            ]
        };
        let head = |kind: &str| {
            format!(
                "kind = \"{kind}\"\ndisplay_name = \"{kind}\"\nversion = \"1\"\nmin_engine = 1\n"
            )
        };
        // A terminal agent whose session id the node chooses, and resumes with it.
        let mut resume = probe("resume-{pid}.json");
        resume.extend(["--resume".to_string(), "{session_id}".to_string()]);
        std::fs::write(
            root.join("manifests/probe.toml"),
            format!(
                "{}[process]\nlaunch = {}\n[resume]\nargv = {}\n[session]\nid_args = [\"--session-id\", \"{{session_id}}\"]\n[[launch.option]]\nid = \"skip\"\nlabel = \"option-skip-permissions\"\nargs = [\"--yolo\"]\nskips_permissions = true\n",
                head("probe").replace("min_engine = 1", "min_engine = 2"),
                q(probe("start-{pid}.json")),
                q(resume)
            ),
        )
        .unwrap();
        // ADR-0060 Amendment A: a kind whose manifest says how to carry on: the probe of that command is another file, so the test can tell
        // a resume that carried the sentence from one that did not.
        let mut carry_resume = probe("resume-{pid}.json");
        carry_resume.extend(["--resume".to_string(), "{session_id}".to_string()]);
        let mut carry_argv = probe("carry-{pid}.json");
        carry_argv.extend([
            "--resume".to_string(),
            "{session_id}".to_string(),
            "{continue}".to_string(),
        ]);
        std::fs::write(
            root.join("manifests/carry.toml"),
            format!(
                "{}[process]\nlaunch = {}\n[resume]\nargv = {}\ncontinue_argv = {}\ncontinue = \"{SENTENCE}\"\n[session]\nid_args = [\"--session-id\", \"{{session_id}}\"]\n",
                head("carry").replace("min_engine = 1", "min_engine = 2"),
                q(probe("start-{pid}.json")),
                q(carry_resume),
                q(carry_argv)
            ),
        )
        .unwrap();
        // A terminal agent the node learns no session of: only Start again.
        std::fs::write(
            root.join("manifests/plain.toml"),
            format!(
                "{}[process]\nlaunch = {}\n",
                head("plain"),
                q(probe("plain-{pid}.json"))
            ),
        )
        .unwrap();
        // An adapter agent: its session comes from the adapter (`session/reported`).
        std::fs::write(
            root.join("manifests/adapter.toml"),
            format!(
                "{}[adapter]\ncommand = {}\n",
                head("adapter"),
                q(vec![fake(), "--adapter".into()])
            ),
        )
        .unwrap();
        let doing = Arc::new(Mutex::new(String::from("setting up")));
        let watchdog = Some(Watchdog::arm(root.clone(), Arc::clone(&doing)));
        let mut e = Self {
            root,
            node: None,
            resume_all: false,
            doing,
            watchdog,
        };
        e.start();
        e
    }

    /// Says what the test does next, for [`Watchdog`].
    fn doing(&self, what: impl Into<String>) {
        *self.doing.lock().unwrap_or_else(|e| e.into_inner()) = what.into();
    }

    fn start(&mut self) {
        self.doing("stopping the node");
        self.node = None;
        self.doing("starting the node");
        self.node = Some(
            Node::start(&NodeConfig {
                resume_all: self.resume_all,
                node_id: "node-resume".into(),
                socket: self.root.join("n.sock"),
                data_dir: self.root.join("data"),
                worktree_root: self.root.join("worktrees"),
                manifests_dir: self.root.join("manifests"),
                projects: vec![],
                limits: NodeLimits::default(),
            })
            .unwrap(),
        );
    }

    fn node(&self) -> &Node {
        self.node.as_ref().unwrap()
    }

    fn client(&self) -> Client {
        Client::connect(self.node().socket().unwrap()).unwrap()
    }

    fn folder(&self) -> PathBuf {
        self.root.join("work/a")
    }

    fn launch(&self, kind: &str) -> String {
        self.launch_with(kind, &[])
    }

    fn launch_with(&self, kind: &str, options: &[&str]) -> String {
        self.doing(format!("launching {kind}"));
        let mut c = self.client();
        let info = match c
            .call(Cmd::DirectoryInspect(v1::DirectoryInspect {
                path: self.folder().to_string_lossy().into_owned(),
                ..Default::default()
            }))
            .unwrap()
            .unwrap()
            .result
        {
            Some(Ok_::DirectoryInfo(i)) => i,
            other => panic!("{other:?}"),
        };
        match c
            .call(Cmd::AgentLaunch(v1::AgentLaunch {
                path: info.path,
                project_id: info.project_id,
                kind: kind.into(),
                launch_options: options.iter().map(|o| o.to_string()).collect(),
                confirmed_launch_options: options.iter().map(|o| o.to_string()).collect(),
                ..Default::default()
            }))
            .unwrap()
            .unwrap()
            .result
        {
            Some(Ok_::AgentLaunch(r)) => r.agent_id,
            other => panic!("{other:?}"),
        }
    }

    fn resume(&self, id: &str, new_session: bool) -> Result<v1::AgentLaunchResult, String> {
        self.doing(format!("resuming {id} (new session: {new_session})"));
        match self
            .client()
            .call(Cmd::AgentResume(v1::AgentResume {
                agent_id: id.into(),
                new_session,
                authorization: None,
            }))
            .unwrap()
        {
            Ok(v1::CommandOk {
                result: Some(Ok_::AgentLaunch(r)),
            }) => Ok(r),
            Ok(other) => panic!("{other:?}"),
            Err(e) => Err(e.code),
        }
    }

    /// The ledger is told the agent's state, as the node's event path does while it runs.
    fn set_state(&self, id: &str, state: v1::AgentState) {
        self.node().ledger.observe(
            &v1::event::Body::AgentStateChanged(v1::AgentStateChanged {
                agent_id: id.into(),
                state: state as i32,
                reported: false,
            }),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        );
        assert_eq!(
            self.node().ledger.get(id).unwrap().last_state,
            match state {
                v1::AgentState::Working => "working",
                v1::AgentState::Blocked => "blocked",
                v1::AgentState::Idle => "idle",
                _ => "unknown",
            }
        );
    }

    /// The `args` the fake agent wrote to the probe files whose names start with `prefix`.
    fn probes(&self, prefix: &str) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        for f in std::fs::read_dir(&self.root).unwrap().flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            if name.starts_with(prefix) && name.ends_with(".json") {
                let v: serde_json::Value =
                    serde_json::from_str(&std::fs::read_to_string(f.path()).unwrap()).unwrap();
                out.push(
                    v["args"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|a| a.as_str().unwrap().to_string())
                        .collect(),
                );
            }
        }
        out
    }

    fn running(&self, id: &str) -> bool {
        self.node()
            .supervisor
            .lock()
            .unwrap()
            .list()
            .iter()
            .any(|v| v.id.as_str() == id && v.running)
    }

    fn session(&self, id: &str) -> String {
        self.node()
            .ledger
            .get(id)
            .map(|e| e.session_id)
            .unwrap_or_default()
    }

    fn wait(&self, what: &str, ok: impl Fn(&Self) -> bool) {
        self.doing(format!("waiting for {what}"));
        let end = Instant::now() + Duration::from_secs(60);
        while !ok(self) {
            assert!(
                Instant::now() < end,
                "timed out waiting for {what}\n{}",
                self.state()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// What a failed wait saw, so a failure on a runner nobody can log in to is diagnosable from its log: the supervisor's agents, the
    /// ledger's entry for each (with the process it holds, and what the system says of that pid now), the ledger file, the probe
    /// reports and the folder's files.
    fn state(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let Some(node) = self.node.as_ref() else {
            return "no node".into();
        };
        let views = node
            .supervisor
            .try_lock()
            .map(|s| s.list())
            .unwrap_or_default();
        let _ = writeln!(out, "supervisor agents: {}", views.len());
        for v in &views {
            let _ = writeln!(out, "  {v:?}");
        }
        for pid in node
            .supervisor
            .try_lock()
            .map(|s| s.agent_pids())
            .unwrap_or_default()
        {
            let _ = writeln!(
                out,
                "  pid of {}: {} -> process_identity {:?}",
                pid.0.as_str(),
                pid.1,
                agora_platform::process_identity(pid.1)
            );
        }
        for v in &views {
            let _ = writeln!(out, "ledger entry: {:?}", node.ledger.get(v.id.as_str()));
        }
        out.push_str(&folder_state(&self.root));
        out
    }

    /// The arguments of the start report that passed `session` as the new session id, once one is written. Found by what it says,
    /// never by how many reports there are: a report is named by its pid, and a later process can get an earlier one's (Windows).
    /// Waits for the start that passed `session` and returns its arguments as read that time: reading again can miss a report
    /// a moment later (on Windows a file being read by another check, or rewritten, is skipped), so the found value is kept.
    fn wait_start(&self, what: &str, session: &str) -> Vec<String> {
        let found = std::cell::RefCell::new(None);
        self.wait(what, |e| match e.start_with_session(session) {
            Some(a) => {
                *found.borrow_mut() = Some(a);
                true
            }
            None => false,
        });
        found.into_inner().unwrap_or_default()
    }

    fn start_with_session(&self, session: &str) -> Option<Vec<String>> {
        self.reports("start-").iter().map(args).find(|a| {
            a.windows(2)
                .any(|w| w[0] == "--session-id" && w[1] == session)
        })
    }

    /// The probe reports (`<prefix>-<pid>.json`) written so far.
    fn reports(&self, prefix: &str) -> Vec<serde_json::Value> {
        std::fs::read_dir(&self.root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
            .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
            .collect()
    }

    fn interrupted(&self) -> Vec<v1::AgentInterrupted> {
        self.node()
            .events
            .events_after(0)
            .into_iter()
            .filter_map(|e| match e.body {
                Some(v1::event::Body::AgentInterrupted(i)) => Some(i),
                _ => None,
            })
            .collect()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        // A failed test ends its own agents first, without the node, so a node stuck in a call cannot keep them (or its own stop)
        // waiting. Its `Watchdog` stays armed until the end: a stop that hangs anyway still ends the run.
        if std::thread::panicking() {
            let pids: Vec<u32> = self
                .node
                .as_ref()
                .and_then(|n| n.supervisor.try_lock().ok().map(|s| s.agent_pids()))
                .unwrap_or_default()
                .into_iter()
                .map(|(_, pid)| pid)
                .collect();
            let _ = end_scratch_agents(Some(&pids));
        }
        self.doing("stopping the node at the end");
        self.node = None;
        let _ = std::fs::remove_dir_all(&self.root);
        drop(self.watchdog.take());
    }
}

fn args(report: &serde_json::Value) -> Vec<String> {
    report["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn agents_interrupted_by_a_node_restart_resume_under_the_same_id_with_the_same_session() {
    if !supported() {
        return;
    }
    let mut e = Env::new("restart");
    let adapter = e.launch("adapter");
    let probe = e.launch("probe");
    let plain = e.launch("plain");
    assert_eq!(
        (adapter.as_str(), probe.as_str(), plain.as_str()),
        ("agent-1", "agent-2", "agent-3")
    );
    // The adapter's own session, and the one the node chose for the probe (passed as `--session-id`).
    e.wait("the adapter's session", |e| {
        e.session(&adapter) == "native-s-1"
    });
    let chosen = e.session(&probe);
    assert_eq!(chosen.len(), 36, "{chosen}");
    e.wait("the probe's start", |e| !e.reports("start-").is_empty());
    let first = args(&e.reports("start-")[0]);
    assert!(
        first
            .windows(2)
            .any(|w| w[0] == "--session-id" && w[1] == chosen),
        "{first:?}"
    );
    assert!(e.session(&plain).is_empty());

    // The machine stops: the node goes, its agents die with it. A new node starts on the same data folder.
    e.start();
    let found = e.interrupted();
    let by = |id: &str| found.iter().find(|i| i.agent_id == id).cloned().unwrap();
    assert!(by(&adapter).resumable && by(&probe).resumable && !by(&plain).resumable);
    assert_eq!(
        std::fs::canonicalize(&by(&probe).folder).unwrap(),
        std::fs::canonicalize(e.folder()).unwrap()
    );
    assert!(!e.running(&adapter) && !e.running(&probe));

    // Resume: same id, same folder, same session.
    let r = e.resume(&adapter, false).unwrap();
    assert_eq!(r.agent_id, adapter);
    assert!(e.running(&adapter));
    e.wait("the adapter's session again", |e| {
        e.node()
            .ledger
            .get(&adapter)
            .is_some_and(|x| !x.interrupted())
    });
    assert_eq!(e.session(&adapter), "native-s-1");

    let r = e.resume(&probe, false).unwrap();
    assert_eq!(r.agent_id, probe);
    assert!(!r.terminal_id.is_empty());
    e.wait("the probe's resume", |e| !e.reports("resume-").is_empty());
    let report = &e.reports("resume-")[0];
    let resumed = args(report);
    assert!(
        resumed
            .windows(2)
            .any(|w| w[0] == "--resume" && w[1] == chosen),
        "{resumed:?}"
    );
    // A resume is not a first launch: no new session id.
    assert!(!resumed.iter().any(|a| a == "--session-id"), "{resumed:?}");
    assert_eq!(
        std::fs::canonicalize(report["cwd"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(e.folder()).unwrap()
    );
    assert_eq!(e.session(&probe), chosen);

    // Twice is refused; without a session only Start again.
    assert_eq!(
        e.resume(&probe, false).unwrap_err(),
        "agent.not_interrupted"
    );
    assert_eq!(
        e.resume(&plain, false).unwrap_err(),
        "agent.session_missing"
    );
    assert_eq!(
        e.resume("agent-77", false).unwrap_err(),
        "agent.not_interrupted"
    );
    let r = e.resume(&plain, true).unwrap();
    assert_eq!(r.agent_id, plain);
    assert!(e.running(&plain));

    // An id is never given twice: the next agent is agent-4.
    assert_eq!(e.launch("plain"), "agent-4");
}

#[test]
fn a_folder_that_is_gone_is_not_resumed_and_the_owner_can_resume_on_their_own() {
    if !supported() {
        return;
    }
    let mut e = Env::new("folder");
    let adapter = e.launch("adapter");
    e.wait("the adapter's session", |e| {
        e.session(&adapter) == "native-s-1"
    });
    let probe = e.launch("probe");
    e.start();
    // On Windows a folder cannot be removed while a process that is being ended still has it as its working folder: try for a while.
    let end = Instant::now() + Duration::from_secs(60);
    while let Err(err) = std::fs::remove_dir_all(e.folder()) {
        assert!(Instant::now() < end, "{err}");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(e.resume(&probe, false).unwrap_err(), "agent.folder_changed");
    assert_eq!(e.resume(&probe, true).unwrap_err(), "agent.folder_changed");
    assert!(e.node().ledger.get(&probe).unwrap().interrupted());

    // `resume = auto` for the adapter kind in that folder (made again): the next start brings it back by itself, and not the probe.
    std::fs::create_dir_all(e.folder()).unwrap();
    std::fs::write(
        e.root.join("data/policy"),
        format!(
            "resume = auto\nresume.kinds = adapter\nresume.folders = {}\n",
            e.root.join("work").display()
        ),
    )
    .unwrap();
    e.start();
    assert!(e.running(&adapter));
    assert!(!e.running(&probe));
    assert!(e.node().ledger.get(&probe).unwrap().interrupted());
}

#[test]
fn a_helper_is_resumed_only_while_its_parent_runs() {
    if !supported() {
        return;
    }
    let mut e = Env::new("helper");
    let parent = e.launch("plain");
    let helper = e.launch("plain");
    // The ledger as it would be after an agent_start: the helper names its parent (written by the node's own events).
    e.node()
        .events
        .record_bodies(vec![v1::event::Body::AgentStarted(v1::AgentStarted {
            agent_id: helper.clone(),
            project_id: e.node().ledger.get(&helper).unwrap().project_id,
            node_id: "node-resume".into(),
            worktree_id: String::new(),
            kind: "plain".into(),
            level: 2,
            parent_agent_id: parent.clone(),
            ..Default::default()
        })]);
    e.start();
    assert_eq!(
        e.resume(&helper, true).unwrap_err(),
        "agent.parent_not_running"
    );
    e.resume(&parent, true).unwrap();
    let r = e.resume(&helper, true).unwrap();
    assert_eq!(r.agent_id, helper);
    let views = e.node().supervisor.lock().unwrap().list();
    let v = views.iter().find(|v| v.id.as_str() == helper).unwrap();
    assert_eq!(v.parent.as_ref().map(|p| p.as_str()), Some(parent.as_str()));
    assert!(Path::new(&r.path).is_dir());
}

const BAD_IDS: &[&str] = &[
    "--dangerously-bypass-approvals-and-sandbox",
    "-x",
    "a b",
    "a;b",
    "ünïcode",
    "",
];

#[test]
fn start_again_gives_a_kind_that_chooses_its_ids_a_new_one_it_can_resume_later() {
    if !supported() {
        return;
    }
    let mut e = Env::new("again");
    let probe = e.launch("probe");
    // The ledger may learn the chosen session on a later tick (#696).
    e.wait("the first session", |e| !e.session(&probe).is_empty());
    let first = e.session(&probe);
    // #1011: started for real before the node stops. On Windows a node that stopped at once ended the probe before it wrote its
    // report, and a wait for two reports then waited for one that never came.
    e.wait("the first start", |e| {
        e.start_with_session(&first).is_some()
    });
    e.start();
    let r = e.resume(&probe, true).unwrap();
    assert_eq!(r.agent_id, probe);
    // A new id, passed as at a first launch, and kept as its session.
    e.wait("the new session", |e| {
        let s = e.session(&probe);
        !s.is_empty() && s != first
    });
    let fresh = e.session(&probe);
    assert_eq!(fresh.len(), 36);
    e.wait("the second start", |e| {
        e.start_with_session(&fresh).is_some()
    });
    // So after the next restart it can be resumed with it.
    e.start();
    assert!(e
        .interrupted()
        .iter()
        .any(|i| i.agent_id == probe && i.resumable));
    e.resume(&probe, false).unwrap();
    e.wait("the resume", |e| !e.reports("resume-").is_empty());
    assert!(args(&e.reports("resume-")[0])
        .windows(2)
        .any(|w| w[0] == "--resume" && w[1] == fresh));
}

#[test]
fn a_session_id_that_could_be_an_option_never_reaches_a_command_line() {
    if !supported() {
        return;
    }
    let long = "a".repeat(129);
    let mut bad: Vec<&str> = BAD_IDS.to_vec();
    bad.push(&long);
    let mut e = Env::new("inject");
    let probe = e.launch("probe");
    // (d) A hook or file reporting one for a terminal agent: refused.
    for id in &bad {
        let set = e
            .node()
            .supervisor
            .lock()
            .unwrap()
            .report_session(&agora_core::AgentId::new(&probe), id);
        assert!(set.is_err(), "{id:?}");
    }
    // Recorded as reported anyway (a forged event): the ledger does not keep it.
    for id in &bad {
        e.node()
            .events
            .record_bodies(vec![v1::event::Body::AgentSessionReported(
                v1::AgentSessionReported {
                    agent_id: probe.clone(),
                    session_id: id.to_string(),
                },
            )]);
        assert!(e.session(&probe).is_empty(), "{id:?}");
    }
    // (b) Written into the ledger file while the node is down: dropped as it loads, so (c) Resume is refused and nothing starts.
    e.node = None;
    let file = e.root.join("data/sessions/ledger.json");
    let mut ledger: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    for entry in ledger["entries"].as_array_mut().unwrap() {
        entry["session_id"] = serde_json::Value::from("--dangerously-bypass-approvals-and-sandbox");
    }
    std::fs::write(&file, serde_json::to_vec(&ledger).unwrap()).unwrap();
    e.start();
    assert!(e
        .interrupted()
        .iter()
        .any(|i| i.agent_id == probe && !i.resumable));
    assert_eq!(
        e.resume(&probe, false).unwrap_err(),
        "agent.session_missing"
    );
    assert!(e.reports("resume-").is_empty());
}

fn stop(e: &Env, id: &str, helpers: bool) -> Result<(), String> {
    e.doing(format!("stopping {id}"));
    e.client()
        .call(Cmd::AgentStop(v1::AgentStop {
            agent_id: id.into(),
            stop_helpers: helpers,
        }))
        .unwrap()
        .map(|_| ())
        .map_err(|err| err.code)
}

#[test]
fn an_agent_that_outlived_a_crashed_node_is_stopped_before_it_is_resumed() {
    // Where an agent can outlive its node at all (on Windows its job object ends it with the node, and no identity is kept).
    if !supported() || agora_platform::process_identity(std::process::id()).is_err() {
        return;
    }
    let mut e = Env::new("crash");
    let probe = e.launch("probe");
    e.wait("the probe's process in the ledger", |e| {
        e.node()
            .ledger
            .get(&probe)
            .is_some_and(|x| x.process().is_some())
    });
    let old = e.node().ledger.get(&probe).unwrap().process().unwrap();
    // Another agent, which the ledger will say the first one started (a helper, ADR-0057).
    let helper = e.launch("probe");
    e.wait("the helper's process in the ledger", |e| {
        e.node()
            .ledger
            .get(&helper)
            .is_some_and(|x| x.process().is_some())
    });
    let helper_process = e.node().ledger.get(&helper).unwrap().process().unwrap();
    // The node is killed without stopping its agents (kill -9): the probe keeps running.
    e.node.take().unwrap().crash();
    // A pid that was given to another process since: this test's own, with another start time.
    let file = e.root.join("data/sessions/ledger.json");
    let mut ledger: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    ledger["entries"].as_array_mut().unwrap().push(serde_json::json!({
        "agent_id": "agent-50", "kind": "plain", "project_id": "p", "folder": "/nowhere", "directory": true,
        "started_at_ms": 1, "pid": std::process::id(), "process_group": std::process::id(), "process_started": 1
    }));
    for entry in ledger["entries"].as_array_mut().unwrap() {
        if entry["agent_id"] == helper.as_str() {
            entry["parent_agent_id"] = serde_json::Value::from(probe.as_str());
        }
    }
    std::fs::write(&file, serde_json::to_vec(&ledger).unwrap()).unwrap();
    e.start();
    assert!(agora_platform::process_still_running(&old));
    assert!(agora_platform::process_still_running(&helper_process));
    let found = e.interrupted();
    assert!(found.iter().any(|i| i.agent_id == probe && i.still_running));
    assert!(found
        .iter()
        .any(|i| i.agent_id == "agent-50" && !i.still_running));
    // Two processes on one conversation are never started: Resume waits until it is stopped.
    assert_eq!(e.resume(&probe, false).unwrap_err(), "agent.still_running");
    assert_eq!(e.resume(&probe, true).unwrap_err(), "agent.still_running");
    // A reused pid is never touched: stopping that entry signals nothing (this test is still here to say so) and only forgets it.
    stop(&e, "agent-50", false).unwrap();
    assert!(e.node().ledger.get("agent-50").unwrap().process().is_none());
    // Stop ends exactly the old process, and with stop_helpers its helper that outlived the node too; then Resume works.
    stop(&e, &probe, true).unwrap();
    e.wait("the helper's old process to end", |_| {
        !agora_platform::process_still_running(&helper_process)
    });
    e.wait("the old process to end", |_| {
        !agora_platform::process_still_running(&old)
    });
    assert!(e
        .interrupted()
        .iter()
        .any(|i| i.agent_id == probe && !i.still_running));
    let r = e.resume(&probe, false).unwrap();
    assert_eq!(r.agent_id, probe);
    assert!(e.running(&probe));
}

/// Codex review of #437 (`docs/reviews/0437-ended-survivor.md`): a survivor that ends after it was announced still running is cleared
/// by Stop (nothing is signalled), and the window is told it can be resumed again.
#[test]
fn review_a_survivor_that_ended_after_announcement_can_be_cleared_by_stop() {
    if !supported() || agora_platform::process_identity(std::process::id()).is_err() {
        return;
    }
    let mut e = Env::new("review-dead-survivor");
    let probe = e.launch("probe");
    e.wait("the owned probe process in the ledger", |e| {
        e.node()
            .ledger
            .get(&probe)
            .is_some_and(|x| x.process().is_some())
    });
    let process = e.node().ledger.get(&probe).unwrap().process().unwrap();
    e.node.take().unwrap().crash();
    e.start();
    assert!(e
        .interrupted()
        .iter()
        .any(|i| i.agent_id == probe && i.still_running));
    // End only this test's fake process after the hub would have seen still_running=true.
    agora_platform::kill_process_if_same(&process).unwrap();
    e.wait("the owned probe to end", |_| {
        !agora_platform::process_still_running(&process)
    });
    let result = stop(&e, &probe, false);
    assert!(
        result.is_ok(),
        "Stop it cannot clear an already-ended survivor: {result:?}"
    );
    // The last word on it says it no longer runs, and Resume works.
    let last = e
        .interrupted()
        .into_iter()
        .rfind(|i| i.agent_id == probe)
        .unwrap();
    assert!(!last.still_running);
    assert!(e.node().ledger.get(&probe).unwrap().process().is_none());
    e.resume(&probe, false).unwrap();
    // Unknown agents are still refused.
    assert!(stop(&e, "agent-77", false).is_err());
}

/// ADR-0060 with ADR-0059: a resumed agent asks before acting again; the options it was started with are kept in the ledger, never
/// applied again.
#[test]
fn a_resumed_agent_never_gets_its_dont_ask_option_back() {
    if !supported() {
        return;
    }
    let mut e = Env::new("options");
    let probe = e.launch_with("probe", &["skip"]);
    // The ledger may learn the chosen session on a later tick (#696).
    e.wait("the first session", |e| !e.session(&probe).is_empty());
    let first = e.session(&probe);
    assert!(e
        .wait_start("the probe's start", &first)
        .iter()
        .any(|a| a == "--yolo"));
    e.wait("the options in the ledger", |e| {
        e.node()
            .ledger
            .get(&probe)
            .is_some_and(|x| x.launch_options == ["skip"])
    });
    e.start();
    e.resume(&probe, false).unwrap();
    e.wait("the resume", |e| !e.reports("resume-").is_empty());
    assert!(!args(&e.reports("resume-")[0]).iter().any(|a| a == "--yolo"));
    // Start again does not bring it back either. The second start is found by its new session, not by counting reports (#1011).
    e.start();
    e.resume(&probe, true).unwrap();
    e.wait("the new session", |e| {
        let s = e.session(&probe);
        !s.is_empty() && s != first
    });
    let fresh = e.session(&probe);
    assert!(!e
        .wait_start("the second start", &fresh)
        .iter()
        .any(|a| a == "--yolo"));
}

#[test]
fn resume_auto_brings_agents_back_eight_at_a_time_at_most_sixteen_and_not_twice_in_ten_minutes() {
    if !supported() {
        return;
    }
    use agora_node::daemon::{AUTO_RESUME_MAX, AUTO_RESUME_TOTAL};
    // ADR-0062 item 4: `agora node run --resume auto` needs no policy file and covers every kind and folder. At most AUTO_RESUME_MAX at
    // once, the next round a few seconds later, AUTO_RESUME_TOTAL at one start; the rest stay offered. An agent with no session of
    // its own stays offered too.
    let mut e = Env::new("all");
    let probes: Vec<String> = (0..=AUTO_RESUME_TOTAL).map(|_| e.launch("probe")).collect();
    let plain = e.launch("plain");
    for p in &probes {
        e.wait("the probe's session", |e| !e.session(p).is_empty());
    }
    e.resume_all = true;
    e.start();
    let back = |e: &Env| probes.iter().filter(|p| e.running(p)).count();
    assert_eq!(back(&e), AUTO_RESUME_MAX);
    e.wait("the second round", |e| back(e) == AUTO_RESUME_TOTAL);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(back(&e), AUTO_RESUME_TOTAL);
    let last = probes.last().unwrap();
    assert!(!e.running(last));
    assert!(e.node().ledger.get(last).unwrap().interrupted());
    assert!(!e.running(&plain));
    assert!(e.node().ledger.get(&plain).unwrap().interrupted());

    // A restart soon after (a crash loop): the ones just brought back stay offered; only the one never brought back comes back.
    e.start();
    assert!(e.running(last));
    assert_eq!(back(&e), 1);

    // Without it (and without a policy file) nothing comes back by itself.
    e.resume_all = false;
    e.start();
    assert_eq!(back(&e), 0);
}

#[test]
fn the_policy_file_always_wins_over_resume_auto() {
    if !supported() {
        return;
    }
    // ADR-0062 item 4: a `resume` line in the node's own policy file is the stricter rule: `ask` (or any other word) stops the flag,
    // and `auto` keeps it to the kinds and folders it lists.
    let mut e = Env::new("policy");
    e.resume_all = true;
    let p = e.launch("probe");
    e.wait("the probe's session", |e| !e.session(&p).is_empty());
    std::fs::create_dir_all(e.root.join("data")).unwrap();
    for policy in [
        "resume = ask\n".to_string(),
        "resume = forbid\n".to_string(),
        format!(
            "resume = auto\nresume.kinds = adapter\nresume.folders = {}\n",
            e.root.join("work").display()
        ),
    ] {
        std::fs::write(e.root.join("data/policy"), &policy).unwrap();
        for _ in 0..2 {
            e.start();
            assert!(!e.running(&p), "{policy}");
            assert!(e.node().ledger.get(&p).unwrap().interrupted(), "{policy}");
        }
    }
}

/// ADR-0060 Amendment A, a real restart: the node brings back by itself an agent that was working, and its resume carries the kind's
/// sentence as one argument; the ledger says it was sent and a second start does not send it again.
#[test]
fn a_working_agent_brought_back_by_itself_is_given_the_kinds_carry_on_sentence_once() {
    if !supported() {
        return;
    }
    let mut e = Env::new("carry-working");
    let id = e.launch("carry");
    e.wait("the session", |e| !e.session(&id).is_empty());
    e.set_state(&id, v1::AgentState::Working);
    e.resume_all = true;
    e.start();
    e.wait("the agent back", |e| e.running(&id));
    e.wait("the probe", |e| !e.probes("carry-").is_empty());
    let carried = e.probes("carry-");
    assert_eq!(carried.len(), 1, "{carried:?}");
    // The sentence is an argument of its own, after the session id; the plain resume command did not run.
    let sid = e.session(&id);
    let args = &carried[0];
    assert_eq!(
        &args[args.len() - 3..],
        ["--resume", sid.as_str(), SENTENCE]
    );
    assert!(e.probes("resume-").is_empty(), "the plain resume ran too");
    let entry = e.node().ledger.get(&id).unwrap();
    assert!(entry.continue_sent_ms > 0 && entry.auto_resumed_at_ms > 0);
    // Another stop and start soon after: it is not brought back again by itself (a crash loop), so nothing more is sent.
    e.start();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(e.probes("carry-").len(), 1);
}

/// The message is never sent otherwise: not to an agent that waited for a person (it could answer a permission prompt), nor to an idle
/// one, nor when the owner's policy says off, nor when a person pressed Resume.
#[test]
fn the_carry_on_sentence_is_never_sent_unless_the_agent_was_working_and_the_node_resumed_it() {
    // Either command wrote its probe: the assertions then say which one it was.
    let any_probe = |e: &Env| !e.probes("resume-").is_empty() || !e.probes("carry-").is_empty();
    if !supported() {
        return;
    }
    for (name, state) in [
        ("blocked", v1::AgentState::Blocked),
        ("idle", v1::AgentState::Idle),
    ] {
        let mut e = Env::new(&format!("carry-{name}"));
        let id = e.launch("carry");
        e.wait("the session", |e| !e.session(&id).is_empty());
        e.set_state(&id, state);
        e.resume_all = true;
        e.start();
        e.wait("the agent back", |e| e.running(&id));
        e.wait("a probe", |e| any_probe(e));
        assert!(
            e.probes("carry-").is_empty(),
            "{name}: the sentence was sent"
        );
        assert_eq!(e.node().ledger.get(&id).unwrap().continue_sent_ms, 0);
    }
    // The owner's policy says off: resumed by itself, but without the sentence.
    let mut e = Env::new("carry-off");
    let id = e.launch("carry");
    e.wait("the session", |e| !e.session(&id).is_empty());
    e.set_state(&id, v1::AgentState::Working);
    std::fs::create_dir_all(e.root.join("data")).unwrap();
    std::fs::write(e.root.join("data/policy"), "resume.continue = off\n").unwrap();
    e.resume_all = true;
    e.start();
    e.wait("the agent back", |e| e.running(&id));
    e.wait("a probe", |e| any_probe(e));
    assert!(e.probes("carry-").is_empty(), "off: the sentence was sent");
    // A person pressed Resume: they are there, so nothing is sent for them.
    let mut e = Env::new("carry-person");
    let id = e.launch("carry");
    e.wait("the session", |e| !e.session(&id).is_empty());
    e.set_state(&id, v1::AgentState::Working);
    e.start();
    assert!(!e.running(&id));
    e.resume(&id, false).unwrap();
    e.wait("a probe", |e| any_probe(e));
    assert!(
        e.probes("carry-").is_empty(),
        "Resume: the sentence was sent"
    );
    assert_eq!(e.node().ledger.get(&id).unwrap().continue_sent_ms, 0);
    // A kind with no carry-on command in its manifest is resumed as usual, whatever it was doing.
    let mut e = Env::new("carry-none");
    let id = e.launch("probe");
    e.wait("the session", |e| !e.session(&id).is_empty());
    e.set_state(&id, v1::AgentState::Working);
    e.resume_all = true;
    e.start();
    e.wait("the agent back", |e| e.running(&id));
    assert_eq!(e.node().ledger.get(&id).unwrap().continue_sent_ms, 0);
}

/// ADR-0060 Amendment A, the policy line: on by default, on only by the word `on`, and the stricter reading when the file exists but
/// cannot be read (a reviewer found it fell back to on).
#[test]
fn the_carry_on_policy_is_on_by_default_and_off_for_anything_but_on_or_an_unreadable_file() {
    use agora_node::daemon::read_policy;
    let dir = std::env::temp_dir().join(format!("carry-policy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let off = |text: &str| {
        let f = dir.join("policy");
        std::fs::write(&f, text).unwrap();
        read_policy(&f).resume.no_carry_on
    };
    // No file, no line, `on`: carrying on is allowed.
    assert!(!read_policy(&dir.join("missing")).resume.no_carry_on);
    assert!(!off("resume = auto\n"));
    assert!(!off("resume.continue = on\n"));
    assert!(!off("resume.continue =   on  \n"));
    // `off`, any other word, or an empty value: not allowed.
    for text in [
        "resume.continue = off\n",
        "resume.continue = maybe\n",
        "resume.continue = ON\n",
        "resume.continue =\n",
    ] {
        assert!(off(text), "{text:?}");
    }
    // A file that exists and cannot be read (a folder where the file should be): not allowed.
    let unreadable = dir.join("folder");
    std::fs::create_dir_all(&unreadable).unwrap();
    assert!(read_policy(&unreadable).resume.no_carry_on);
    let _ = std::fs::remove_dir_all(&dir);
}
