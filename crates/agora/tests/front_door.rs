//! ADR-0062 rung 1: plain `agora` starts this machine's hub and node detached, enrolls the node with its own hub, and opens the
//! interface; killing the interface changes nothing for the hub, the node or the agents. The agent is `agora-fake-agent`, the home
//! a temporary folder, the hub port one found free from 17900 on (never the real `~/.agora` or a running hub).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

fn agora(home: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(AGORA);
    c.args(args)
        .env("AGORA_HOME", home)
        .env("AGORA_LANG", "en")
        .stdin(Stdio::null());
    c
}

fn run(home: &Path, args: &[&str]) -> (i32, String, String) {
    let out: Output = agora(home, args).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A short folder: the node's socket path must stay under ~100 bytes.
fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agfd-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
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
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

fn fake_agent() -> PathBuf {
    let mut fake = PathBuf::from(AGORA);
    fake.pop();
    fake.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    static FRESH: std::sync::Once = std::sync::Once::new();
    FRESH.call_once(|| {
        assert!(
            Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
                .args(["build", "-q", "-p", "agora-fake-agent"])
                .status()
                .unwrap()
                .success()
        );
    });
    fake
}

/// A free port from `from` on; each test has its own range, as the tests run at the same time.
fn free_port(from: u16) -> u16 {
    (from..from + 500)
        .find(|p| {
            std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok()
                && std::net::TcpStream::connect(("127.0.0.1", *p)).is_err()
        })
        .expect("a free port")
}

fn toml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Whatever the test does, the daemons it started are stopped at the end.
struct Down(PathBuf);

/// The interface process, killed when a failing test unwinds too.
struct Client(std::process::Child);

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for Down {
    fn drop(&mut self) {
        let _ = agora(&self.0, &["down"]).output();
    }
}

/// Whether one row of `agora status` starts with `start` (#935: "not connected to wss://..." contains "connected to wss://...", so a
/// substring test took a node still dialling for one that is connected).
fn has_row(status: &str, start: &str) -> bool {
    status.lines().any(|l| l.trim_start().starts_with(start))
}

fn wait_for(what: &str, limit: Duration, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + limit;
    while !f() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn version_prints_the_version_and_needs_no_setup() {
    let home = tmp("version");
    for flag in ["--version", "-V", "version"] {
        let (code, out, _) = run(&home, &[flag]);
        assert_eq!(code, 0);
        assert!(
            out.contains(&format!("Agora {}", env!("CARGO_PKG_VERSION"))),
            "{out}"
        );
    }
    assert!(!home.join("machine.toml").exists());
}

#[test]
fn help_prints_usage_and_a_first_run_without_a_terminal_explains_itself() {
    let home = tmp("first");
    let (code, out, _) = run(&home, &["--help"]);
    assert_eq!(code, 0);
    assert!(out.contains("agora [--ui tui|web|none [--save]]"), "{out}");

    let (code, _, err) = run(&home, &[]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("not set up"), "{err}");
    assert!(!home.join("machine.toml").exists());

    let (code, _, err) = run(&home, &["status"]);
    assert_eq!(code, 1);
    assert!(err.contains("Run `agora` first"), "{err}");
    let (code, _, _) = run(&home, &["--ui", "desktop"]);
    assert_eq!(code, 2);
    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn agora_brings_the_machine_up_and_killing_the_interface_leaves_agents_running() {
    if !agora_platform::PTY_SUPPORTED || !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let home = tmp("up");
    let repo = home.join("proj");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    std::fs::create_dir_all(home.join("manifests")).unwrap();
    let probe = home.join("probe-{pid}.json");
    std::fs::write(
        home.join("manifests/probe.toml"),
        format!(
            "kind = \"probe\"\ndisplay_name = \"Probe\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [{}, \"--probe-directory\", {}]\n",
            toml_string(&fake_agent().to_string_lossy()),
            toml_string(&probe.to_string_lossy()),
        ),
    )
    .unwrap();
    let port = free_port(17900);
    std::fs::write(
        home.join("machine.toml"),
        format!(
            "roles = [\"hub\", \"node\"]\nui = \"tui\"\nhub_listen = \"127.0.0.1:{port}\"\nhub_names = [\"localhost\"]\nhub_dir = {}\nnode_projects = [{}]\n",
            toml_string(&home.join("hub").to_string_lossy()),
            toml_string(&repo.to_string_lossy()),
        ),
    )
    .unwrap();
    let _down = Down(home.clone());

    // The interface: plain `agora`, its output going to a file (no terminal, so the terminal client prints its plain list).
    let out = std::fs::File::create(home.join("client.out")).unwrap();
    let mut client = Client(
        agora(&home, &[])
            .stdout(out.try_clone().unwrap())
            .stderr(out)
            .spawn()
            .unwrap(),
    );
    // The node's row says it is connected, as this machine's node: "connected to <hub> as <id>".
    let linked = format!("connected to wss://localhost:{port} as ");
    let mut s = String::new();
    wait_for("the hub and the node", Duration::from_secs(90), || {
        s = run(&home, &["status"]).1;
        s.contains("The hub is running (process")
            && s.contains("The node is running (process")
            && has_row(&s, &linked)
    });
    // Negative control: the status that says the link is up says nothing of it being down, and the old wording for the configured
    // hub is not used.
    assert!(!s.contains("not connected"), "{s}");
    // The client opened the terminal client once the node showed up at the hub: it connected without any code typed.
    wait_for("the interface", Duration::from_secs(30), || {
        std::fs::read_to_string(home.join("client.out"))
            .is_ok_and(|t| t.contains("Connecting") || t.contains("machine"))
    });

    let (code, out, err) = run(
        &home,
        &["agent", "start", "probe", "--cwd", &repo.to_string_lossy()],
    );
    assert_eq!(code, 0, "{out}{err}");
    let mut agent_pid = 0u32;
    wait_for("the agent", Duration::from_secs(30), || {
        agent_pid = std::fs::read_dir(&home)
            .unwrap()
            .filter_map(|e| {
                let n = e.ok()?.file_name().to_string_lossy().into_owned();
                n.strip_prefix("probe-")?
                    .strip_suffix(".json")?
                    .parse()
                    .ok()
            })
            .next()
            .unwrap_or(0);
        agent_pid != 0
    });
    let locks =
        |role: &str| std::fs::read_to_string(home.join(format!("run/{role}.lock"))).unwrap();
    let (hub_lock, node_lock) = (locks("hub"), locks("node"));

    // Kill the interface outright (SIGKILL on Unix): nothing below it notices.
    client.0.kill().unwrap();
    client.0.wait().unwrap();
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        agora_platform::process_start_time(agent_pid).is_some(),
        "the agent ended with the interface"
    );
    let (code, list, _) = run(&home, &["agent", "list"]);
    assert_eq!(code, 0);
    assert!(list.contains("running"), "{list}");
    let (_, s, _) = run(&home, &["status"]);
    assert!(s.contains("agents running: 1"), "{s}");
    assert!(s.contains("logs"), "{s}");

    // A second `agora` finds them running and starts nothing new.
    let (code, out, err) = run(&home, &["--ui", "none"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("Agora is running"), "{out}");
    assert!(!out.contains("Starting"), "{out}");
    assert_eq!((locks("hub"), locks("node")), (hub_lock, node_lock));
    // `--ui` alone applies to this run only; with `--save` it becomes the default.
    let saved = || std::fs::read_to_string(home.join("machine.toml")).unwrap();
    assert!(saved().contains("ui = \"tui\""), "{}", saved());
    let (code, out, err) = run(&home, &["--ui", "none", "--save"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(saved().contains("ui = \"none\""), "{}", saved());

    // `agora down` stops the node, then the hub.
    let (code, out, err) = run(&home, &["down"]);
    assert_eq!(code, 0, "{out}{err}");
    let (node_at, hub_at) = (
        out.find("Stopped the node").expect(&out),
        out.find("Stopped the hub").expect(&out),
    );
    assert!(node_at < hub_at, "{out}");
    assert!(!home.join("run/hub.lock").exists() && !home.join("run/node.lock").exists());
    let (_, s, _) = run(&home, &["status"]);
    assert!(
        s.contains("The hub is not running") && s.contains("The node is not running"),
        "{s}"
    );
    if agora_platform::HOLDER_BUILT {
        // ADR-0079: the agent runs in the terminal holder, so plain `agora down` leaves it running, says so, and `status` still counts it.
        assert!(out.contains("1 agent keeps running"), "{out}");
        assert!(
            agora_platform::process_start_time(agent_pid).is_some(),
            "plain `agora down` ended the agent"
        );
        assert!(
            s.contains("agents still running without the node: 1"),
            "{s}"
        );
        let (code, out, err) = run(&home, &["down", "--agents"]);
        assert_eq!(code, 0, "{out}{err}");
        assert!(out.contains("Stopped 1 agent"), "{out}");
        assert!(
            agora_platform::process_start_time(agent_pid).is_none(),
            "`agora down --agents` left the agent running"
        );
    }
    let (_, out, _) = run(&home, &["down"]);
    assert!(out.contains("Nothing to stop"), "{out}");
    let _ = std::fs::remove_dir_all(home);
}

/// `agora status` said "connected to <hub>" for the hub a machine was configured for while every dial failed (found on a Mac whose
/// local network permission was gone). The node here is enrolled with a hub address that refuses connections: status must say it is
/// not connected, with the reason, and never use the "connected to" wording.
#[test]
fn status_says_not_connected_while_the_hub_refuses_the_node() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let home = tmp("closed-hub");
    let port = free_port(18900);
    std::fs::write(
        home.join("machine.toml"),
        "roles = [\"node\"]\nui = \"none\"\nnode_projects = []\n",
    )
    .unwrap();
    std::fs::create_dir_all(home.join("link")).unwrap();
    for (name, text) in [
        ("url", format!("wss://127.0.0.1:{port}")),
        ("node-id", "mac".to_string()),
        ("ca.pem", String::new()),
        ("cert.pem", String::new()),
        ("key.pem", String::new()),
    ] {
        std::fs::write(home.join("link").join(name), text).unwrap();
    }
    let _down = Down(home.clone());
    let (code, out, err) = run(&home, &[]);
    assert_eq!(code, 0, "{out}{err}");
    let mut last = String::new();
    let hub = format!("not connected to wss://127.0.0.1:{port}");
    wait_for(
        "status to say not connected",
        Duration::from_secs(60),
        || {
            last = run(&home, &["status"]).1;
            last.contains("The node is running (process") && has_row(&last, &hub)
        },
    );
    // The reason is shown once a dial has failed; Windows words a refusal differently from Unix, so only there it is not pinned.
    if !cfg!(windows) {
        wait_for(
            "status to name the refusal",
            Duration::from_secs(30),
            || {
                last = run(&home, &["status"]).1;
                has_row(&last, &format!("{hub}: nothing is listening"))
            },
        );
    }
    assert!(!has_row(&last, "connected to wss://"), "{last}");
    assert!(has_row(&last, &hub), "{last}");
    assert!(!last.contains(" as mac"), "{last}");
    let _ = run(&home, &["down"]);
    let _ = std::fs::remove_dir_all(home);
}

/// A caller that captures `agora`'s output (a script, `$(agora)`, CI) reads to the end of the pipe: the daemons `agora` starts must
/// not hold it, or the caller waits until they end.
#[test]
fn a_caller_capturing_the_output_is_not_kept_waiting_by_the_daemons() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let home = tmp("pipe");
    let port = free_port(18400);
    std::fs::write(
        home.join("machine.toml"),
        format!(
            "roles = [\"hub\", \"node\"]\nui = \"none\"\nhub_listen = \"127.0.0.1:{port}\"\nhub_names = [\"localhost\"]\nhub_dir = {}\nnode_projects = []\n",
            toml_string(&home.join("hub").to_string_lossy()),
        ),
    )
    .unwrap();
    let _down = Down(home.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    let h = home.clone();
    // `output()` returns only when the pipes reach their end: with a daemon holding them it would wait for `agora down`.
    std::thread::spawn(move || {
        let _ = tx.send(agora(&h, &[]).output());
    });
    let out = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("`agora` with captured output did not return while its daemons run")
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("Agora is running"), "{text}");
    let (_, s, _) = run(&home, &["status"]);
    assert!(s.contains("The hub is running (process"), "{s}");
    let (code, _, _) = run(&home, &["down"]);
    assert_eq!(code, 0);
    let _ = std::fs::remove_dir_all(home);
}

/// The process holding a role's lock (its supervisor), if the lock is there.
fn lock_pid(home: &Path, role: &str) -> Option<u32> {
    std::fs::read_to_string(home.join(format!("run/{role}.lock")))
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// The role process its supervisor runs: the supervisor's `agora` child.
fn supervised_child(supervisor: u32) -> Option<u32> {
    agora_platform::process_children(supervisor)
        .ok()?
        .into_iter()
        .find(|c| {
            agora_platform::process_name(*c).is_ok_and(|n| n.to_lowercase().starts_with("agora"))
        })
}

fn alive(pid: u32) -> bool {
    agora_platform::process_start_time(pid).is_some()
}

/// Kills `pid` outright, so it runs no code of its own on the way out: `SIGKILL` on Unix, `TerminateProcess` on Windows.
fn kill_outright(pid: u32) {
    if cfg!(unix) {
        assert!(Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .unwrap()
            .success());
    } else {
        agora_platform::terminate_process(pid).unwrap();
    }
}

/// ADR-0062 item 8: the front door starts the node under a supervisor holding its lock. A node that crashes is started again after
/// 30 s (and `agora status` says so); a second supervisor on the same folder refuses; a supervisor killed outright takes its node
/// with it (no orphan); `agora down` stops the supervisor and the node in one command.
#[test]
fn the_supervisor_restarts_a_crashed_node_and_never_leaves_it_behind() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let home = tmp("sup");
    let repo = home.join("proj");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    std::fs::write(
        home.join("machine.toml"),
        format!(
            "roles = [\"node\"]\nui = \"none\"\nnode_projects = [{}]\n",
            toml_string(&repo.to_string_lossy()),
        ),
    )
    .unwrap();
    let _down = Down(home.clone());
    let up = |what: &str| {
        let (code, out, err) = run(&home, &[]);
        assert_eq!(code, 0, "{what}: {out}{err}");
        let supervisor = lock_pid(&home, "node").expect("a node lock");
        let mut node = None;
        wait_for(
            "the node under its supervisor",
            Duration::from_secs(20),
            || {
                node = supervised_child(supervisor);
                node.is_some()
            },
        );
        (supervisor, node.unwrap())
    };

    let (supervisor, node) = up("first start");
    let (_, s, _) = run(&home, &["status"]);
    assert!(
        s.contains(&format!("The node is running (process {node}, version"))
            && s.contains(&format!("its supervisor is process {supervisor})")),
        "{s}"
    );

    // A second supervisor on this folder finds the lock held and ends without starting anything.
    let (code, _, err) = run(&home, &["supervise", "node"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("already looks after the node"), "{err}");
    assert_eq!(supervised_child(supervisor), Some(node));

    // A crash (after the 10 s that would make it a refusal) is followed by a restart 30 s later.
    std::thread::sleep(Duration::from_secs(11));
    agora_platform::terminate_process(node).unwrap();
    wait_for("the restart notice", Duration::from_secs(10), || {
        let s = run(&home, &["status"]).1;
        s.contains("The node is not running: it crashed")
            && s.contains(&format!(
                "its supervisor (process {supervisor}) starts it again"
            ))
    });
    let mut again = None;
    wait_for("the restarted node", Duration::from_secs(60), || {
        again = supervised_child(supervisor).filter(|p| *p != node);
        again.is_some() && run(&home, &["agent", "list"]).0 == 0
    });
    let again = again.unwrap();
    let (_, s, _) = run(&home, &["status"]);
    assert!(s.contains("restarted once in the last 10 minutes"), "{s}");
    let log = std::fs::read_to_string(home.join("logs/node.log")).unwrap();
    assert!(
        log.contains("starting it again in 30 s (restart 1 of 5"),
        "{log}"
    );

    // `agora down` while the supervisor waits to restart a second crash: both end at once and nothing starts again.
    std::thread::sleep(Duration::from_secs(11));
    agora_platform::terminate_process(again).unwrap();
    // The second crash's own notice: until the supervisor has seen this crash, the record still ends with the first one's restart,
    // so "it crashed" alone can be the first crash's line, and `agora down` then comes before the crash is logged (#935, macOS).
    wait_for("the second restart notice", Duration::from_secs(10), || {
        let s = run(&home, &["status"]).1;
        s.contains("The node is not running: it crashed") && s.contains("(restart 2 of 5)")
    });
    let stopping = Instant::now();
    let (code, out, err) = run(&home, &["down"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(!alive(supervisor), "the supervisor outlived `agora down`");
    assert!(stopping.elapsed() < Duration::from_secs(10));
    assert!(!home.join("run/node.lock").exists());

    // The supervisor killed outright: its node does not outlive it.
    let (supervisor, again) = up("start after the stop during the wait");
    kill_outright(supervisor);
    wait_for(
        "the node to end with its supervisor",
        Duration::from_secs(10),
        || !alive(again),
    );

    // `agora` starts both again; `agora down` stops them in one command.
    let (supervisor, node) = up("start after the kill");
    let (code, out, err) = run(&home, &["down"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.contains(&format!(
            "Stopped the node and its supervisor (process {supervisor})"
        )),
        "{out}"
    );
    wait_for(
        "the node to end with `agora down`",
        Duration::from_secs(10),
        || !alive(node) && !alive(supervisor),
    );

    // Linux's M1 on #543: a supervisor still waiting for its front door when `agora down` freed the lock never takes the freed lock and
    // starts the role again (the front door that started it is gone here, as is the lock).
    let (code, _, err) = run(&home, &["supervise", "node", "--handoff", "1:1"]);
    assert_eq!(code, 1, "{err}");
    assert!(
        err.contains("was stopped while its supervisor was starting"),
        "{err}"
    );
    assert!(!home.join("run/node.lock").exists());
    let (_, s, _) = run(&home, &["status"]);
    assert!(s.contains("The node is not running"), "{s}");
    let log = std::fs::read_to_string(home.join("logs/node.log")).unwrap();
    assert_eq!(
        log.matches("Supervisor: the node ended after").count(),
        2,
        "the kill and `agora down` are not crashes: {log}"
    );
    let _ = std::fs::remove_dir_all(home);
}

/// Linux's follow-up to #543: `agora down` racing supervisors started by hand (a service manager starts them without a front door).
/// Whatever the timing, nothing may keep running unseen: every supervisor left after `agora down` holds the lock (so `agora status`
/// shows it and the next `agora down` stops it), and `agora down` never deletes a lock another supervisor took meanwhile.
#[test]
fn agora_down_racing_supervisors_never_leaves_one_running_unseen() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let home = tmp("race");
    let repo = home.join("proj");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    std::fs::write(
        home.join("machine.toml"),
        format!(
            "roles = [\"node\"]\nui = \"none\"\nnode_projects = [{}]\n",
            toml_string(&repo.to_string_lossy()),
        ),
    )
    .unwrap();
    let _down = Down(home.clone());
    for round in 0..6 {
        let mut supervisors: Vec<std::process::Child> = (0..2)
            .map(|_| {
                agora(&home, &["supervise", "node"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap()
            })
            .collect();
        // Vary the moment `agora down` comes in: before, during and after the supervisors take the lock.
        std::thread::sleep(Duration::from_millis(30 * round));
        let (code, out, err) = run(&home, &["down"]);
        assert_eq!(code, 0, "round {round}: {out}{err}");
        // Let the late ones settle (refuse, or start their node).
        std::thread::sleep(Duration::from_secs(3));
        for c in &mut supervisors {
            if c.try_wait().unwrap().is_none() {
                assert_eq!(
                    lock_pid(&home, "node"),
                    Some(c.id()),
                    "round {round}: supervisor {} runs without holding the lock ({out})",
                    c.id()
                );
            }
        }
        // The next `agora down` stops whatever is left, node included.
        let (code, out, err) = run(&home, &["down"]);
        assert_eq!(code, 0, "round {round}: {out}{err}");
        for c in &mut supervisors {
            let end = Instant::now() + Duration::from_secs(10);
            while c.try_wait().unwrap().is_none() {
                assert!(
                    Instant::now() < end,
                    "round {round}: supervisor {} outlived two `agora down`",
                    c.id()
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        assert!(
            !home.join("run/node.lock").exists(),
            "round {round}: a lock is left"
        );
        let (_, s, _) = run(&home, &["status"]);
        assert!(s.contains("The node is not running"), "round {round}: {s}");
    }
    let _ = std::fs::remove_dir_all(home);
}

/// ADR-0062 item 6: a service manager starts `agora supervise <role> --home <folder> --log` with nothing else set (a Windows task cannot
/// set `AGORA_HOME` and redirects nothing). `--home` beats the environment, `--log` sends the supervisor's notes and the role's own
/// output to the role's log, and `agora down` stops it like any supervisor.
#[test]
fn a_service_started_supervisor_finds_its_folder_and_writes_its_log() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let home = tmp("svc");
    let repo = home.join("proj");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    std::fs::write(
        home.join("machine.toml"),
        format!(
            "roles = [\"node\"]\nui = \"none\"\nnode_projects = [{}]\n",
            toml_string(&repo.to_string_lossy()),
        ),
    )
    .unwrap();
    let _down = Down(home.clone());
    // The environment names another folder: `--home` must win. Output goes nowhere: `--log` must write the log itself.
    let elsewhere = tmp("svc-elsewhere");
    let mut supervisor = agora(&elsewhere, &[])
        .args(["supervise", "node", "--home"])
        .arg(&home)
        .arg("--log")
        .current_dir(&elsewhere)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for(
        "the node under the service's supervisor",
        Duration::from_secs(20),
        || {
            lock_pid(&home, "node") == Some(supervisor.id())
                && supervised_child(supervisor.id()).is_some()
                && run(&home, &["agent", "list"]).0 == 0
        },
    );
    assert!(
        !elsewhere.join("run").exists(),
        "it used the environment's folder"
    );
    let log = std::fs::read_to_string(home.join("logs/node.log")).unwrap();
    assert!(log.contains("Node is running"), "{log}");
    let (code, out, err) = run(&home, &["down"]);
    assert_eq!(code, 0, "{out}{err}");
    let end = Instant::now() + Duration::from_secs(10);
    while supervisor.try_wait().unwrap().is_none() {
        assert!(Instant::now() < end, "the supervisor outlived `agora down`");
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = std::fs::remove_dir_all(elsewhere);
    let _ = std::fs::remove_dir_all(home);
}

/// Linux's decision on #563: under a service (`--log`) every end the supervisor decides itself exits 0, so the service manager restarts
/// only a crash of the supervisor; without `--log` (the front door's supervisor) the code says the role did not start.
#[test]
fn a_service_supervisor_exits_0_for_its_own_decisions() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let home = tmp("exit0");
    // A project folder that does not exist: the node refuses at once.
    std::fs::write(
        home.join("machine.toml"),
        format!(
            "roles = [\"node\"]\nui = \"none\"\nnode_projects = [{}]\n",
            toml_string(&home.join("missing").to_string_lossy()),
        ),
    )
    .unwrap();
    let _down = Down(home.clone());
    let home_arg = home.to_string_lossy().into_owned();
    // A refusal: 0 under a service, non-zero for the front door's supervisor.
    let (code, out, err) = run(&home, &["supervise", "node", "--home", &home_arg, "--log"]);
    assert_eq!(code, 0, "{out}{err}");
    let log = std::fs::read_to_string(home.join("logs/node.log")).unwrap();
    assert!(log.contains("too soon to be a crash"), "{log}");
    let (code, _, err) = run(&home, &["supervise", "node"]);
    assert_ne!(code, 0, "{err}");

    // A lock another live process holds: 0 under a service, 1 otherwise.
    let mut holder = agora(&home, &["supervise", "node", "--home", &home_arg, "--log"]);
    std::fs::write(
        home.join("machine.toml"),
        "roles = [\"node\"]\nui = \"none\"\nnode_projects = []\n",
    )
    .unwrap();
    let mut holder = holder
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for(
        "the first supervisor's lock",
        Duration::from_secs(20),
        || lock_pid(&home, "node") == Some(holder.id()),
    );
    let (code, _, err) = run(&home, &["supervise", "node", "--home", &home_arg, "--log"]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("already looks after the node"), "{err}");
    let (code, _, err) = run(&home, &["supervise", "node"]);
    assert_eq!(code, 1, "{err}");
    let (code, out, err) = run(&home, &["down"]);
    assert_eq!(code, 0, "{out}{err}");
    let end = Instant::now() + Duration::from_secs(10);
    while holder.try_wait().unwrap().is_none() {
        assert!(Instant::now() < end, "the supervisor outlived `agora down`");
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = std::fs::remove_dir_all(home);
}

/// #556: the front door's own log is opened as the supervisor's is. A `logs` folder that is a link to another folder is refused with
/// the log named: nothing is written there and nothing in it is rotated, and nothing starts.
#[test]
fn a_linked_logs_folder_is_refused_by_the_front_door() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let home = tmp("loglink");
    let repo = home.join("proj");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    std::fs::write(
        home.join("machine.toml"),
        format!(
            "roles = [\"node\"]\nui = \"none\"\nnode_projects = [{}]\n",
            toml_string(&repo.to_string_lossy()),
        ),
    )
    .unwrap();
    let _down = Down(home.clone());
    let elsewhere = home.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    // Over the rotation limit, so a rotation through the link would rename it.
    let big = "x".repeat(6 * 1024 * 1024);
    std::fs::write(elsewhere.join("node.log"), &big).unwrap();
    agora_platform::link_dir(&elsewhere, &home.join("logs")).unwrap();
    let (code, out, err) = run(&home, &[]);
    assert_ne!(code, 0, "{out}{err}");
    assert!(err.contains("node.log"), "{err}");
    assert!(err.contains("is a link"), "{err}");
    assert_eq!(
        std::fs::metadata(elsewhere.join("node.log")).unwrap().len(),
        big.len() as u64,
        "the log in the link's target was written to or rotated"
    );
    assert!(!elsewhere.join("node.log.1").exists());
    assert!(lock_pid(&home, "node").is_none_or(|p| !alive(p)));
    let _ = std::fs::remove_dir_all(home);
}

/// ADR-0104: the turn `agent` said `word` in, as written in its node's transcript file (the agent's own entry, not the person's).
fn written_answer(home: &Path, agent: &str, word: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(home.join(format!("data/transcripts/{agent}.jsonl")))
    else {
        return false;
    };
    text.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .any(|v| {
            v["entry"]["text"] == word
                && v["entry"]["role"] == agora_protocol::v1::ChatRole::Agent as i32
        })
}

/// The usage a stopped node logged for `agent`: (input, output) summed over its `AgentUsageReported` events.
fn logged_usage(home: &Path, agent: &str) -> (u64, u64) {
    let events =
        agora_node::daemon::EventSink::open_with_cap(&home.join("data/events"), "local", 1 << 30)
            .expect("the node's event log");
    events
        .events_after(0)
        .iter()
        .filter_map(|e| match &e.body {
            Some(agora_protocol::v1::event::Body::AgentUsageReported(u)) if u.agent_id == agent => {
                Some((u.input_tokens, u.output_tokens))
            }
            _ => None,
        })
        .fold((0, 0), |(i, o), (a, b)| (i + a, o + b))
}

/// A home whose machine runs a node only, for the project `proj` (a Git repository), with the agent kind `k`: the fake adapter.
fn node_home_with_an_adapter_kind(tag: &str) -> (PathBuf, PathBuf) {
    let home = tmp(tag);
    let repo = home.join("proj");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    std::fs::create_dir_all(home.join("manifests")).unwrap();
    std::fs::write(
        home.join("manifests/k.toml"),
        format!(
            "kind = \"k\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{}, \"--adapter\"]\n",
            toml_string(&fake_agent().to_string_lossy()),
        ),
    )
    .unwrap();
    std::fs::write(
        home.join("machine.toml"),
        format!(
            "roles = [\"node\"]\nui = \"none\"\nnode_projects = [{}]\n",
            toml_string(&repo.to_string_lossy()),
        ),
    )
    .unwrap();
    (home, repo)
}

/// However a test ends, the agents the holder keeps are stopped too (`agora down` stops only the node).
struct DownAgents(PathBuf);

impl Drop for DownAgents {
    fn drop(&mut self) {
        let _ = agora(&self.0, &["down", "--agents"]).output();
    }
}

/// The agent kind `slow`, an adapter that never answers (it would wait 60 s), in `home`. Dropped, it ends the adapter that was started
/// (the holder runs it in a session of its own, so it outlives the node and the holder): it writes its pid where this finds it.
#[cfg(unix)]
struct SlowKind(PathBuf);

#[cfg(unix)]
impl SlowKind {
    fn add(home: &Path) -> SlowKind {
        let pid_file = home.join("slow.pid");
        std::fs::write(
            home.join("manifests/slow.toml"),
            format!(
                "kind = \"slow\"\ndisplay_name = \"Slow\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [\"/bin/sh\", \"-c\", {}]\n",
                toml_string(&format!("echo $$ > '{}'; exec sleep 60", pid_file.display())),
            ),
        )
        .unwrap();
        SlowKind(pid_file)
    }
}

#[cfg(unix)]
impl Drop for SlowKind {
    fn drop(&mut self) {
        if let Some(pid) = std::fs::read_to_string(&self.0)
            .ok()
            .and_then(|t| t.trim().parse::<u32>().ok())
        {
            let _ = agora_platform::terminate_process(pid);
        }
    }
}

/// The agent id `agora agent start` printed.
fn agent_id_in(out: &str) -> String {
    out.split_whitespace()
        .find(|w| w.starts_with("agent-"))
        .expect("an agent id")
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '-')
        .to_string()
}

/// ADR-0104 O2: `agora down` stops the node cleanly. Its structured agent, which the terminal holder runs, keeps running, and what the
/// node still held of the turn it just finished is on disk after the stop: the answer in the conversation's file and the usage the
/// node gathers for 5 s before it logs it (lost on every planned stop before, which ended the node like a kill). Then the same with a
/// holder that does not answer (stopped by the test): the node still stops within its deadline and `agora down` succeeds.
#[test]
fn agora_down_stops_the_node_cleanly_and_its_kept_agent_loses_nothing() {
    if !cfg!(unix) || !agora_platform::HOLDER_BUILT || !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let (home, repo) = node_home_with_an_adapter_kind("o2");
    let _down = DownAgents(home.clone());
    let up = || {
        let (code, out, err) = run(&home, &[]);
        assert_eq!(code, 0, "{out}{err}");
        wait_for("the node", Duration::from_secs(30), || {
            run(&home, &["agent", "list"]).0 == 0
        });
    };
    // Stops the node with `agora down`, which must succeed within its own wait, and say the agent keeps running.
    let down = || {
        let stopping = Instant::now();
        let (code, out, err) = run(&home, &["down"]);
        assert_eq!(code, 0, "{out}{err}");
        assert!(out.contains("1 agent keeps running"), "{out}");
        assert!(stopping.elapsed() < Duration::from_secs(10));
        assert!(!home.join("run/node.lock").exists());
    };
    // A turn, waited for until its answer is in the transcript file (the writer runs once a second); its usage is still being gathered.
    let turn = |id: &str, word: &str| {
        let (code, out, err) = run(&home, &["agent", "prompt", id, word]);
        assert_eq!(code, 0, "{out}{err}");
        wait_for("the answer on disk", Duration::from_secs(20), || {
            written_answer(&home, id, word)
        });
    };

    up();
    let (code, out, err) = run(
        &home,
        &["agent", "start", "k", "--cwd", &repo.to_string_lossy()],
    );
    assert_eq!(code, 0, "{out}{err}");
    let id = agent_id_in(&out);
    turn(&id, "first");
    down();
    assert_eq!(
        logged_usage(&home, &id),
        (10, 5),
        "the usage the node was still gathering when it stopped"
    );

    // The next node takes the agent up; this time its holder does not answer when the node stops.
    up();
    turn(&id, "second");
    let holder: u32 = std::fs::read_to_string(home.join("run/hold.pid"))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let pause = |signal: &str| {
        assert!(Command::new("kill")
            .args([signal, &holder.to_string()])
            .status()
            .unwrap()
            .success());
    };
    pause("-STOP");
    struct Resume(u32);
    impl Drop for Resume {
        fn drop(&mut self) {
            let _ = Command::new("kill")
                .args(["-CONT", &self.0.to_string()])
                .status();
        }
    }
    let resume = Resume(holder);
    std::thread::sleep(Duration::from_millis(300));
    down();
    drop(resume);
    assert_eq!(logged_usage(&home, &id), (20, 11));
    let (code, out, err) = run(&home, &["down", "--agents"]);
    assert_eq!(code, 0, "{out}{err}");
    let _ = std::fs::remove_dir_all(home);
}

/// Sends `signal` (a `kill` name such as `TERM`) to a process this test started.
#[cfg(unix)]
fn signal(pid: u32, signal: &str) {
    assert!(Command::new("kill")
        .args([&format!("-{signal}"), &pid.to_string()])
        .status()
        .unwrap()
        .success());
}

/// Waits up to `limit` for `child` to end; how it ended and how long that took.
#[cfg(unix)]
fn ended_within(
    child: &mut std::process::Child,
    limit: Duration,
) -> Option<(std::process::ExitStatus, Duration)> {
    let began = Instant::now();
    while began.elapsed() < limit {
        if let Some(status) = child.try_wait().unwrap() {
            return Some((status, began.elapsed()));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

/// ADR-0104 O2 (review of #1108): a stop request while the node is still starting (here it waits for a link folder it cannot read
/// yet, which it tries again for 15 s) ends it at once, as before the clean stop: there is nothing to keep yet.
#[cfg(unix)]
#[test]
fn a_node_stopped_while_it_starts_ends_at_once() {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::ExitStatusExt;
    let home = tmp("o2-start");
    let link = home.join("link");
    std::fs::create_dir_all(&link).unwrap();
    std::fs::set_permissions(&link, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read_dir(&link).is_ok() {
        // A user who can read it anyway (root): there is no slow start to stop here.
        let _ = std::fs::set_permissions(&link, std::fs::Permissions::from_mode(0o700));
        return;
    }
    let mut node = agora(&home, &["node", "run"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_secs(1));
    assert!(node.try_wait().unwrap().is_none(), "the node did not wait");
    signal(node.id(), "TERM");
    let ended = ended_within(&mut node, Duration::from_secs(10));
    let _ = node.kill();
    let _ = std::fs::set_permissions(&link, std::fs::Permissions::from_mode(0o700));
    let (status, took) = ended.expect("the node did not end");
    assert_eq!(status.signal(), Some(15), "{status:?}");
    assert!(took < Duration::from_secs(2), "it took {took:?}");
    let _ = std::fs::remove_dir_all(home);
}

/// ADR-0104 O2 (review of #1108): a stop request the node was started ignoring (`nohup`, a background job) stays ignored, as before
/// the clean stop; the others still stop it cleanly, and it ends as that signal ends it.
#[cfg(unix)]
#[test]
fn a_node_started_ignoring_hangups_keeps_ignoring_them() {
    use std::os::unix::process::ExitStatusExt;
    let home = tmp("o2-nohup");
    let mut node = Command::new("sh")
        .args(["-c", "trap '' HUP; exec \"$0\" node run", AGORA])
        .env("AGORA_HOME", &home)
        .env("AGORA_LANG", "en")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for("the node's socket", Duration::from_secs(60), || {
        home.join("node.sock").exists()
    });
    signal(node.id(), "HUP");
    std::thread::sleep(Duration::from_secs(1));
    let still = node.try_wait().unwrap();
    if still.is_none() {
        signal(node.id(), "TERM");
    }
    let ended = ended_within(&mut node, Duration::from_secs(10));
    let _ = node.kill();
    assert!(still.is_none(), "a hangup it ignored stopped it: {still:?}");
    let (status, _) = ended.expect("the node did not end");
    assert_eq!(status.signal(), Some(15), "{status:?}");
    let _ = std::fs::remove_dir_all(home);
}

/// #1119 item 4: one stop that reaches the node twice at once (Ctrl-C on a foreground `agora supervise node` signals the node and its
/// supervisor, which passes its own on as `SIGTERM`) is one request: the node stops cleanly and ends as the first signal ends it. Before,
/// the second ended it at once, as a second request does, and the clean stop was skipped.
///
/// The stop is made to take a while (an agent is being started: its adapter never answers `initialize`, and the stop waits up to 2 s
/// for the supervisor), so the second signal comes while it runs.
#[cfg(unix)]
#[test]
fn a_stop_that_reaches_the_node_twice_at_once_still_stops_it_cleanly() {
    use std::os::unix::process::ExitStatusExt;
    if !agora_platform::HOLDER_BUILT || !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let (home, repo) = node_home_with_an_adapter_kind("o2-echo");
    let _slow = SlowKind::add(&home);
    let _down = DownAgents(home.clone());
    let mut node = agora(&home, &["node", "run"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for("the node", Duration::from_secs(60), || {
        run(&home, &["agent", "list"]).0 == 0
    });
    let mut starting = agora(
        &home,
        &["agent", "start", "slow", "--cwd", &repo.to_string_lossy()],
    )
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    // The start is waiting for the adapter's answer, under the supervisor's lock.
    std::thread::sleep(Duration::from_millis(1500));
    let pid = node.id().to_string();
    // A moment between them, so the first is taken first: two standard signals pending together are delivered lowest number first
    // (`SIGINT` before `SIGTERM` on Linux). Well under the second that makes them one stop, and the stop takes longer.
    assert!(Command::new("sh")
        .args(["-c", "kill -TERM \"$0\"; sleep 0.2; kill -INT \"$0\"", &pid])
        .status()
        .unwrap()
        .success());
    let ended = ended_within(&mut node, Duration::from_secs(10));
    let _ = node.kill();
    let _ = starting.kill();
    let _ = starting.wait();
    let (status, took) = ended.expect("the node did not end");
    assert_eq!(
        status.signal(),
        Some(15),
        "the second signal ended the node at once: {status:?} after {took:?}"
    );
    drop(_down);
    drop(_slow);
    let _ = std::fs::remove_dir_all(home);
}

/// The agent's entry in the node's session ledger, read from its file.
#[cfg(unix)]
fn ledger_entry(home: &Path, id: &str) -> Option<serde_json::Value> {
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(home.join("data/sessions/ledger.json")).ok()?)
            .ok()?;
    v["entries"]
        .as_array()?
        .iter()
        .find(|e| e["agent_id"] == id)
        .cloned()
}

/// The first number of a `pid start ...` file under `run/` (a role's lock, the holder's note).
#[cfg(unix)]
fn pid_in(home: &Path, file: &str) -> u32 {
    std::fs::read_to_string(home.join("run").join(file))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

/// Herdr gap G11: an OS shutdown or reboot. A login session's end (or launchd's, or systemd's for a node started outside its unit)
/// signals every process of the session at once: the node's supervisor (which passes it on to the node), the agents and the terminal
/// holder. An agent's end could then reach the node before it acted on its own stop (it polls for the request), and was recorded as the
/// agent having ended: after the reboot it was not offered to Resume. The rule itself is pinned by the ledger's unit test (this order
/// rarely hit the window before); this is the whole path. An end the node sees once a stop request has come is not an end: both
/// agents (a terminal one and a structured one) are interrupted at the next start, Resume brings each back in its own session, and what
/// the node wrote before (the conversation, the ledger) is all there.
#[cfg(unix)]
#[test]
fn an_os_shutdown_that_stops_the_node_and_its_agents_at_once_leaves_them_to_resume() {
    if !agora_platform::HOLDER_BUILT || !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let (home, repo) = node_home_with_an_adapter_kind("g11");
    // A terminal agent that resumes its own session, and says how it was started (`--resume <session>` when resumed).
    let probe = |file: &str| {
        vec![
            fake_agent().to_string_lossy().into_owned(),
            "--probe-directory".into(),
            home.join(file).to_string_lossy().into_owned(),
        ]
    };
    let mut resume = probe("resume-{pid}.json");
    resume.extend(["--resume".to_string(), "{session_id}".to_string()]);
    let q = |v: Vec<String>| serde_json::to_string(&v).unwrap();
    std::fs::write(
        home.join("manifests/probe.toml"),
        format!(
            "kind = \"probe\"\ndisplay_name = \"probe\"\nversion = \"1\"\nmin_engine = 2\n[process]\nlaunch = {}\n[resume]\nargv = {}\n[session]\nid_args = [\"--session-id\", \"{{session_id}}\"]\n",
            q(probe("start-{pid}.json")),
            q(resume)
        ),
    )
    .unwrap();
    let _down = DownAgents(home.clone());
    let up = || {
        let (code, out, err) = run(&home, &[]);
        assert_eq!(code, 0, "{out}{err}");
        wait_for("the node", Duration::from_secs(30), || {
            run(&home, &["agent", "list"]).0 == 0
        });
    };
    let start = |kind: &str| {
        let (code, out, err) = run(
            &home,
            &["agent", "start", kind, "--cwd", &repo.to_string_lossy()],
        );
        assert_eq!(code, 0, "{out}{err}");
        agent_id_in(&out)
    };
    let pid_of = |id: &str| {
        let mut pid = 0;
        wait_for(
            "the agent's process in the ledger",
            Duration::from_secs(20),
            || {
                pid = ledger_entry(&home, id)
                    .and_then(|e| e["pid"].as_u64())
                    .unwrap_or(0);
                pid > 0 && ledger_entry(&home, id).is_some_and(|e| e["session_id"] != "")
            },
        );
        pid
    };
    up();
    let structured = start("k");
    let terminal = start("probe");
    let (code, out, err) = run(&home, &["agent", "prompt", &structured, "first"]);
    assert_eq!(code, 0, "{out}{err}");
    wait_for("the answer on disk", Duration::from_secs(20), || {
        written_answer(&home, &structured, "first")
    });
    let agents = [pid_of(&structured), pid_of(&terminal)];
    let supervisor = pid_in(&home, "node.lock");
    let holder = pid_in(&home, "hold.pid");
    // The shutdown, in the order that let the agents' ends reach the node before it acted on its own stop: the node's supervisor (which
    // passes it on to the node), a moment later the agents, then the holder (processes this test started, never any other).
    assert!(Command::new("sh")
        .args([
            "-c",
            "kill -TERM \"$0\"; sleep 0.02; kill -TERM \"$1\" \"$2\"; sleep 0.3; kill -TERM \"$3\"",
            &supervisor.to_string(),
            &agents[0].to_string(),
            &agents[1].to_string(),
            &holder.to_string(),
        ])
        .status()
        .unwrap()
        .success());
    wait_for("the node's end", Duration::from_secs(20), || {
        agora_platform::process_start_time(supervisor).is_none()
    });
    wait_for("the holder's end", Duration::from_secs(20), || {
        agora_platform::process_start_time(holder).is_none()
    });
    for id in [&structured, &terminal] {
        let entry = ledger_entry(&home, id).unwrap();
        assert_eq!(entry["ended"], false, "the shutdown ended {id}: {entry}");
    }

    // After the reboot: both are offered to Resume, the conversation is there, and Resume brings each back in its own session.
    up();
    let (_, list, _) = run(&home, &["agent", "list"]);
    for id in [&structured, &terminal] {
        let entry = ledger_entry(&home, id).unwrap();
        assert_eq!(entry["ended"], false, "{entry}");
        assert!(entry["interrupted_at_ms"].as_u64().unwrap() > 0, "{entry}");
        assert!(
            list.lines()
                .any(|l| l.starts_with(&format!("{id}\t")) && l.ends_with("\tinterrupted\tresume")),
            "{list}"
        );
        let (code, out, err) = run(&home, &["agent", "resume", id]);
        assert_eq!(code, 0, "{out}{err}");
    }
    assert!(written_answer(&home, &structured, "first"));
    let (code, out, err) = run(&home, &["agent", "prompt", &structured, "again"]);
    assert_eq!(code, 0, "{out}{err}");
    wait_for(
        "the resumed agent's answer",
        Duration::from_secs(20),
        || written_answer(&home, &structured, "again"),
    );
    let session = ledger_entry(&home, &terminal).unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    wait_for(
        "the terminal agent resumed in its session",
        Duration::from_secs(20),
        || {
            std::fs::read_dir(&home).unwrap().flatten().any(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.starts_with("resume-")
                    && name.ends_with(".json")
                    && std::fs::read_to_string(e.path()).is_ok_and(|t| t.contains(&session))
            })
        },
    );
    drop(_down);
    let _ = std::fs::remove_dir_all(home);
}

/// #1119 item 2: `agora down` while an agent is being started. The start holds the node's supervisor while its adapter answers
/// `initialize` (up to 30 s; here a program that never answers). The node's stop cannot have that lock in time, and still writes what
/// does not need it: the usage it was still gathering for another agent's turn. `agora down` succeeds within its wait. Before, the stop
/// waited for the lock until its 5 s deadline ended the process, and the usage was lost.
#[cfg(unix)]
#[test]
fn agora_down_while_an_agent_starts_still_keeps_the_usage_being_gathered() {
    if !agora_platform::HOLDER_BUILT || !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let (home, repo) = node_home_with_an_adapter_kind("o2-busy");
    let _slow = SlowKind::add(&home);
    let _down = DownAgents(home.clone());
    let (code, out, err) = run(&home, &[]);
    assert_eq!(code, 0, "{out}{err}");
    wait_for("the node", Duration::from_secs(30), || {
        run(&home, &["agent", "list"]).0 == 0
    });
    let cwd = repo.to_string_lossy().into_owned();
    let (code, out, err) = run(&home, &["agent", "start", "k", "--cwd", &cwd]);
    assert_eq!(code, 0, "{out}{err}");
    let id = agent_id_in(&out);
    let (code, out, err) = run(&home, &["agent", "prompt", &id, "first"]);
    assert_eq!(code, 0, "{out}{err}");
    wait_for("the answer on disk", Duration::from_secs(20), || {
        written_answer(&home, &id, "first")
    });
    let mut starting = agora(&home, &["agent", "start", "slow", "--cwd", &cwd])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // The start is waiting for the adapter's answer, under the supervisor's lock.
    std::thread::sleep(Duration::from_millis(1500));
    let stopping = Instant::now();
    let (code, out, err) = run(&home, &["down"]);
    let took = stopping.elapsed();
    let _ = starting.kill();
    let _ = starting.wait();
    assert_eq!(code, 0, "{out}{err}");
    assert!(took < Duration::from_secs(10), "{took:?}");
    assert_eq!(
        logged_usage(&home, &id),
        (10, 5),
        "the usage the node was still gathering when it stopped"
    );
    drop(_down);
    drop(_slow);
    let _ = std::fs::remove_dir_all(home);
}
