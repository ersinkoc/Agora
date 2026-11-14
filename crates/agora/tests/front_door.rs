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
    let linked = format!("connected to wss://localhost:{port}");
    wait_for("the hub and the node", Duration::from_secs(90), || {
        let (_, s, _) = run(&home, &["status"]);
        s.contains("The hub is running (process")
            && s.contains("The node is running (process")
            && s.contains(&linked)
    });
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
    let (_, out, _) = run(&home, &["down"]);
    assert!(out.contains("Nothing to stop"), "{out}");
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
        s.contains(&format!("The node is running (process {supervisor}")),
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
        run(&home, &["status"])
            .1
            .contains("restarting after a crash")
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

    // The supervisor killed outright: its node does not outlive it.
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
    assert!(out.contains("Stopped the node"), "{out}");
    wait_for(
        "the node to end with `agora down`",
        Duration::from_secs(10),
        || !alive(node) && !alive(supervisor),
    );
    let log = std::fs::read_to_string(home.join("logs/node.log")).unwrap();
    assert_eq!(
        log.matches("Supervisor: the node ended after").count(),
        1,
        "the kill and `agora down` are not crashes: {log}"
    );
    let _ = std::fs::remove_dir_all(home);
}
