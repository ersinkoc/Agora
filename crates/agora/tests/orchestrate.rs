//! Scripted orchestration from a shell through a real node and real `agora` processes: `agora agent prompt --wait`, `agora agent
//! wait --until` and `agora events`. The agent is the fake L3 adapter (`agora-fake-agent --adapter`), never a real one: `hold:TEXT`
//! keeps it working until a file `go.txt` appears in its folder, `ask` makes it wait for the person (blocked).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

/// `agora-fake-agent` next to the `agora` under test, built once per test binary (a binary left from an older checkout may lack what
/// the tests need).
fn fake_agent() -> String {
    let mut p = PathBuf::from(AGORA);
    p.pop();
    let target = p.parent().map(Path::to_path_buf).unwrap_or_default();
    p.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    static FRESH: std::sync::Once = std::sync::Once::new();
    FRESH.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let mut build = Command::new(cargo);
        build
            .args(["build", "-q", "-p", "agora-fake-agent", "--target-dir"])
            .arg(&target);
        if p.parent().is_some_and(|d| d.ends_with("release")) {
            build.arg("--release");
        }
        assert!(
            build.status().expect("run cargo").success(),
            "building agora-fake-agent failed"
        );
    });
    p.to_string_lossy().into_owned()
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
        .expect("git")
        .success();
    assert!(ok, "git {args:?}");
}

/// A node home with a project `demo` and the fake adapter as kind `fake`, a node running on it, and agent `agent-1` started in
/// `demo` and idle. Declared in that order, so the node (and its agent) ends before the home is removed.
struct Rig {
    node: Option<Child>,
    home: PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        // Its agents are stopped first: an adapter outlives a killed node (ADR-0093), and one in a `hold:` turn would keep running.
        if self.node.is_some() {
            let (_, out, _) = self.cli(&["agent", "list"]);
            for id in out
                .lines()
                .filter(|l| l.ends_with("\trunning"))
                .filter_map(|l| l.split('\t').next())
            {
                let _ = self.cli(&["agent", "stop", id]);
            }
        }
        if let Some(mut n) = self.node.take() {
            let _ = n.kill();
            let _ = n.wait();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

impl Rig {
    fn new(tag: &str) -> Option<Rig> {
        if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
            return None;
        }
        let home = std::env::temp_dir().join(format!("agora-orch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let repo = home.join("demo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
        std::fs::create_dir_all(home.join("manifests")).unwrap();
        let command = serde_json::to_string(&[fake_agent(), "--adapter".into()]).unwrap();
        std::fs::write(
            home.join("manifests/fake.toml"),
            format!("kind = \"fake\"\ndisplay_name = \"Fake\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = {command}\n"),
        )
        .unwrap();
        let mut rig = Rig { node: None, home };
        rig.node = Some(
            Command::new(AGORA)
                .args(["node", "run", "--project"])
                .arg(&repo)
                .env("AGORA_HOME", &rig.home)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("start node"),
        );
        let sock = rig.home.join("node.sock");
        let end = Instant::now() + Duration::from_secs(60);
        while !sock.exists() {
            assert!(Instant::now() < end, "node did not create its socket");
            std::thread::sleep(Duration::from_millis(50));
        }
        let (code, out, err) = rig.cli(&["agent", "start", "fake", "--project", "demo"]);
        assert_eq!(code, 0, "{out}{err}");
        assert!(out.starts_with("Started agent agent-1 in "), "{out}");
        // The adapter has its session once it is idle; a prompt before that is refused.
        let (code, out, err) = rig.cli(&[
            "agent",
            "wait",
            "agent-1",
            "--until",
            "idle",
            "--timeout",
            "60",
        ]);
        assert_eq!((code, out.as_str()), (0, "agent-1\tidle\n"), "{err}");
        Some(rig)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(AGORA);
        c.args(args)
            .current_dir(self.home.join("demo"))
            .env("AGORA_HOME", &self.home)
            .env("AGORA_LANG", "en");
        c
    }

    /// Runs `agora <args>` to its end: (exit code, stdout, stderr).
    fn cli(&self, args: &[&str]) -> (i32, String, String) {
        let out = self.command(args).output().expect("run agora");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn spawn(&self, args: &[&str]) -> Child {
        self.command(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run agora")
    }

    /// Lets the agent's `hold:` turn end.
    fn go(&self) {
        std::fs::write(self.home.join("demo/go.txt"), "go").unwrap();
    }

    fn state(&self) -> String {
        let (_, out, _) = self.cli(&["agent", "list"]);
        out.lines()
            .find(|l| l.starts_with("agent-1\t"))
            .and_then(|l| l.split('\t').nth(2))
            .unwrap_or_default()
            .to_string()
    }

    fn wait_state(&self, want: &str) {
        let end = Instant::now() + Duration::from_secs(30);
        while self.state() != want {
            assert!(Instant::now() < end, "agent-1 never became {want}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Waits for `child` to end (at most `secs`): (exit code, stdout, stderr).
fn ended(mut child: Child, secs: u64) -> (i32, String, String) {
    let end = Instant::now() + Duration::from_secs(secs);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= end {
            let _ = child.kill();
            let out = child.wait_with_output().unwrap();
            panic!(
                "still running after {secs} s; stdout {:?}, stderr {:?}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// `--wait` waits for the turn: it has not returned while the agent works, and returns `idle` with 0 once the turn is over. A
/// `--timeout` shorter than the turn returns 5 with the state at that moment.
#[test]
fn prompt_wait_returns_when_the_turn_is_over_and_a_timeout_says_so() {
    let Some(rig) = Rig::new("idle") else {
        return;
    };
    let child = rig.spawn(&[
        "agent",
        "prompt",
        "agent-1",
        "hold:one",
        "--wait",
        "--timeout",
        "60",
    ]);
    rig.wait_state("working");
    std::thread::sleep(Duration::from_millis(700));
    let mut child = child;
    assert!(
        child.try_wait().unwrap().is_none(),
        "--wait returned while the agent was working"
    );
    rig.go();
    let (code, out, err) = ended(child, 30);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(
        out, "Sent the prompt to agent agent-1\nagent-1\tidle\n",
        "{err}"
    );

    std::fs::remove_file(rig.home.join("demo/go.txt")).unwrap();
    let started = Instant::now();
    let (code, out, err) = rig.cli(&[
        "agent",
        "prompt",
        "agent-1",
        "hold:two",
        "--wait",
        "--timeout",
        "1",
    ]);
    assert_eq!(code, 5, "{out}{err}");
    assert!(out.ends_with("agent-1\tworking\n"), "{out}");
    assert!(
        err.contains("Stopped waiting for agent agent-1 after 1 s; it is Working."),
        "{err}"
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    rig.go();
    rig.wait_state("idle");
}

/// An agent that asks the person ends a `--wait` with 3 (`blocked`); a prompt to it then is refused with `agent.blocked` and 6, and
/// nothing is sent; `wait --until blocked` returns at once with 3, and `wait --until idle --timeout 1` gives up with 5.
#[test]
fn a_blocked_agent_ends_a_wait_with_its_own_code_and_refuses_prompts() {
    let Some(rig) = Rig::new("blocked") else {
        return;
    };
    let (code, out, err) = rig.cli(&[
        "agent",
        "prompt",
        "agent-1",
        "ask",
        "--wait",
        "--timeout",
        "60",
    ]);
    assert_eq!(code, 3, "{out}{err}");
    assert!(out.ends_with("agent-1\tblocked\n"), "{out}");
    assert!(
        err.contains("Agent agent-1 needs you: it is waiting for an answer."),
        "{err}"
    );

    let (code, out, err) = rig.cli(&["agent", "prompt", "agent-1", "hello"]);
    assert_eq!(code, 6, "{out}{err}");
    assert!(out.is_empty(), "nothing was sent: {out}");
    assert!(
        err.contains("agent.blocked")
            && err.contains("Agent agent-1 is waiting for input, so the message was not sent.")
            && err.contains("--force"),
        "{err}"
    );
    let (code, out, err) = rig.cli(&["agent", "prompt", "agent-1", "hello", "--wait"]);
    assert_eq!(code, 6, "{out}{err}");

    let (code, out, err) = rig.cli(&["agent", "wait", "agent-1", "--until", "blocked"]);
    assert_eq!((code, out.as_str()), (3, "agent-1\tblocked\n"), "{err}");
    let (code, out, err) = rig.cli(&[
        "agent",
        "wait",
        "agent-1",
        "--until",
        "idle",
        "--timeout",
        "1",
    ]);
    assert_eq!((code, out.as_str()), (5, "agent-1\tblocked\n"), "{err}");
    assert_eq!(rig.state(), "blocked");
}

/// `wait --until` waits for the states it names (any of them), from a state it is not in; an agent whose process ends ends every
/// wait with 4 (`exited`). Words that are not states are a usage error, and an unknown agent is `agent.not_found`.
#[test]
fn wait_until_returns_on_the_named_states_and_on_an_ended_agent() {
    let Some(rig) = Rig::new("until") else {
        return;
    };
    let waiting = rig.spawn(&[
        "agent",
        "wait",
        "agent-1",
        "--until",
        "working,blocked",
        "--timeout",
        "60",
    ]);
    std::thread::sleep(Duration::from_millis(500));
    let mut waiting = waiting;
    assert!(waiting.try_wait().unwrap().is_none(), "returned on idle");
    let (code, out, err) = rig.cli(&["agent", "prompt", "agent-1", "hold:three"]);
    assert_eq!(code, 0, "{out}{err}");
    let (code, out, err) = ended(waiting, 30);
    assert_eq!((code, out.as_str()), (0, "agent-1\tworking\n"), "{err}");
    rig.go();
    let (code, out, err) = rig.cli(&[
        "agent",
        "wait",
        "agent-1",
        "--until",
        "idle,done",
        "--timeout",
        "30",
    ]);
    assert_eq!((code, out.as_str()), (0, "agent-1\tidle\n"), "{err}");

    let (code, _, err) = rig.cli(&["agent", "wait", "agent-1", "--until", "asleep"]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("--until takes states"), "{err}");
    let (code, _, err) = rig.cli(&["agent", "wait", "agent-9"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("agent.not_found"), "{err}");

    let waiting = rig.spawn(&[
        "agent",
        "wait",
        "agent-1",
        "--until",
        "blocked",
        "--timeout",
        "60",
    ]);
    std::thread::sleep(Duration::from_millis(500));
    let (code, out, err) = rig.cli(&["agent", "stop", "agent-1"]);
    assert_eq!(code, 0, "{out}{err}");
    let (code, out, err) = ended(waiting, 30);
    assert_eq!(code, 4, "{out}{err}");
    assert_eq!(out, "agent-1\texited\n");
    assert!(err.contains("Agent agent-1 is no longer running"), "{err}");
}

/// The lines `agora events` prints, read on a thread so a line that never comes fails the test instead of hanging it.
struct Lines(std::sync::mpsc::Receiver<String>);

impl Lines {
    fn of(out: ChildStdout) -> Lines {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for l in BufReader::new(out).lines() {
                let Ok(l) = l else { break };
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
        Lines(rx)
    }

    fn next(&self) -> String {
        self.0
            .recv_timeout(Duration::from_secs(30))
            .expect("no line within 30 s")
    }
}

/// `agora events --json` prints one JSON object per line: the agents there when it began, then each change; `--agent` keeps one agent's
/// lines.
#[test]
fn events_json_prints_one_object_per_change() {
    let Some(rig) = Rig::new("events") else {
        return;
    };
    let (code, out, err) = rig.cli(&["agent", "start", "fake", "--project", "demo"]);
    assert_eq!(code, 0, "{out}{err}");
    let mut events = rig.spawn(&["events", "--json", "--agent", "agent-1"]);
    let lines = Lines::of(events.stdout.take().unwrap());
    let first: serde_json::Value = serde_json::from_str(&lines.next()).unwrap();
    assert_eq!(first["event"], "present");
    assert_eq!(first["agent_id"], "agent-1");
    assert_eq!(first["kind"], "fake");
    assert_eq!(first["state"], "idle");
    assert_eq!(first["running"], true);
    assert!(first["time_ms"].as_u64().is_some_and(|t| t > 0));

    let (code, out, err) = rig.cli(&["agent", "prompt", "agent-1", "hold:four"]);
    assert_eq!(code, 0, "{out}{err}");
    let working: serde_json::Value = serde_json::from_str(&lines.next()).unwrap();
    assert_eq!(
        (&working["event"], &working["state"], &working["previous"]),
        (&"state".into(), &"working".into(), &"idle".into()),
        "{working}"
    );
    rig.go();
    let idle: serde_json::Value = serde_json::from_str(&lines.next()).unwrap();
    assert_eq!(
        (&idle["event"], &idle["state"], &idle["previous"]),
        (&"state".into(), &"idle".into(), &"working".into()),
        "{idle}"
    );
    let (code, _, err) = rig.cli(&["agent", "stop", "agent-1"]);
    assert_eq!(code, 0, "{err}");
    let stopped: serde_json::Value = serde_json::from_str(&lines.next()).unwrap();
    assert_eq!(
        stopped["agent_id"], "agent-1",
        "agent-2's lines are left out"
    );
    assert!(
        ["exited", "removed"].contains(&stopped["event"].as_str().unwrap()),
        "{stopped}"
    );
    let _ = events.kill();
    let _ = events.wait();
}

/// `agora events | head -1`: once the reader is gone, the next line ends the command with 0 and no panic.
#[test]
fn events_ends_quietly_when_its_reader_goes_away() {
    let Some(rig) = Rig::new("head") else {
        return;
    };
    let mut events = rig.spawn(&["events"]);
    // One line is read, then the pipe is closed (the reading thread ends), as `| head -1` does.
    let out = events.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut l = String::new();
        let _ = BufReader::new(out).read_line(&mut l);
        let _ = tx.send(l);
    });
    let first = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("no line within 30 s");
    assert_eq!(first, "agent-1 (fake): Idle\n");
    let (code, out, err) = rig.cli(&["agent", "prompt", "agent-1", "hold:five"]);
    assert_eq!(code, 0, "{out}{err}");
    rig.wait_state("working");
    let mut stderr = events.stderr.take().unwrap();
    let (code, _, _) = ended(events, 30);
    let mut err = String::new();
    stderr.read_to_string(&mut err).unwrap();
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains("panicked"), "{err}");
    rig.go();
    rig.wait_state("idle");
}
