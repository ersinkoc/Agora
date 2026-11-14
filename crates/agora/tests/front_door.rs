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
    wait_for("the second restart notice", Duration::from_secs(10), || {
        run(&home, &["status"])
            .1
            .contains("The node is not running: it crashed")
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
