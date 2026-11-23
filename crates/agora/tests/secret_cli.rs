//! ADR-0084: `agora node secret set|list|remove` run end to end as real processes: the value comes from standard input and is never
//! echoed, listed, put in an argument, or written by the node into its output. The "agent" is a plain `sh` that dumps its environment.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use agora_platform::{spawn, PtyConfig};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

mod short_socket;

const AGORA: &str = env!("CARGO_BIN_EXE_agora");
const VALUE: &str = "sk-test-0123456789-not-a-real-token";

fn home(name: &str) -> PathBuf {
    let h = std::env::temp_dir().join(format!("agora-secret-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&h);
    std::fs::create_dir_all(h.join("data")).unwrap();
    h
}

/// Runs `agora ARGS` with `stdin` piped in; (exit code, stdout, stderr).
fn run(home: &Path, args: &[&str], stdin: &str) -> (i32, String, String) {
    let mut child = Command::new(AGORA)
        .args(args)
        .env("AGORA_HOME", home)
        .env("AGORA_LANG", "en")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn set_reads_standard_input_and_never_shows_the_value() {
    let h = home("cli");
    let (code, out, err) = run(
        &h,
        &["node", "secret", "set", "MY_TOKEN"],
        &format!("{VALUE}\n"),
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("MY_TOKEN"), "{out}");
    assert!(!out.contains(VALUE) && !err.contains(VALUE));
    // The file holds the value (no newline), owner-only, in a folder only the owner enters.
    let file = h.join("data/secrets/MY_TOKEN");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), VALUE);
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(h.join("data/secrets"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    // Listing: the name, never the value.
    let (code, out, err) = run(&h, &["node", "secret", "list"], "");
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "MY_TOKEN");
    assert!(!out.contains(VALUE) && !err.contains(VALUE));
    // Replacing keeps one entry.
    let (code, _, _) = run(&h, &["node", "secret", "set", "MY_TOKEN"], "other-value\n");
    assert_eq!(code, 0);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "other-value");
    // Removing.
    let (code, out, _) = run(&h, &["node", "secret", "remove", "MY_TOKEN"], "");
    assert_eq!(code, 0, "{out}");
    assert!(!file.exists());
    let (code, _, err) = run(&h, &["node", "secret", "remove", "MY_TOKEN"], "");
    assert_eq!(code, 1, "{err}");
    let (code, out, _) = run(&h, &["node", "secret", "list"], "");
    assert_eq!((code, out.contains("No secret")), (0, true));
    let _ = std::fs::remove_dir_all(&h);
}

#[test]
fn a_value_given_as_an_argument_is_refused_and_bad_input_stores_nothing() {
    let h = home("refused");
    // Negative control: the value on the command line is not accepted, and nothing is stored (it would be in `ps` and the history).
    let (code, out, err) = run(&h, &["node", "secret", "set", "MY_TOKEN", VALUE], "");
    assert_eq!(code, 2);
    assert!(!out.contains(VALUE) && !err.contains(VALUE), "{out}{err}");
    for (name, input) in [
        ("PATH", "x\n"),
        ("CON", "x\n"),
        ("com1", "x\n"),
        ("1BAD", "x\n"),
        ("A=B", "x\n"),
        ("OK_NAME", "\n"),
        ("OK_NAME", ""),
        ("OK_NAME", "two\nlines\n"),
    ] {
        let (code, out, err) = run(&h, &["node", "secret", "set", name], input);
        assert_eq!(code, 1, "{name} {input:?}: {out}{err}");
    }
    let (_, out, _) = run(&h, &["node", "secret", "list"], "");
    assert!(out.contains("No secret"), "{out}");
    // A name that differs only in capitals from a stored one is refused.
    assert_eq!(run(&h, &["node", "secret", "set", "MY_TOKEN"], "a\n").0, 0);
    let (code, _, err) = run(&h, &["node", "secret", "set", "my_token"], "b\n");
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("capital"), "{err}");
    let _ = std::fs::remove_dir_all(&h);
}

struct NodeProc(Child);
impl Drop for NodeProc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
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

const OTHER: &str = "sk-test-OTHER-kind-9876543210-fake";

fn manifest(h: &Path, kind: &str, token: &str, script: &str) {
    let launch = serde_json::to_string(&["sh", "-c", script]).unwrap();
    std::fs::write(
        h.join(format!("manifests/{kind}.toml")),
        format!("kind = \"{kind}\"\ndisplay_name = \"{kind}\"\nversion = \"1\"\nmin_engine = 4\n[process]\nlaunch = {launch}\n[accounts]\ncredential_env = [\"{token}\"]\n"),
    )
    .unwrap();
}

fn repo(h: &Path) -> PathBuf {
    let repo = h.join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "hi").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    std::fs::create_dir_all(h.join("manifests")).unwrap();
    repo
}

/// A node as a real process whose stdout and stderr go to `log` (appended). `extra` is set in its environment.
/// `control_token`: the policy makes this node require the control token (ADR-0082). The node binds the socket before it
/// writes the token beside it (a second node fails at the bind and never replaces the running one's token, control.rs), so
/// such a node is ready for control calls only when both exist; a CLI racing in between reads no token and is refused.
fn start_node(
    h: &Path,
    repo: &Path,
    log: &Path,
    extra: &[(&str, &str)],
    control_token: bool,
) -> NodeProc {
    let _ = std::fs::remove_file(h.join("node.sock"));
    let open = || {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap()
    };
    let mut c = Command::new(AGORA);
    c.args(["node", "run", "--project"])
        .arg(repo)
        .env("AGORA_HOME", h)
        .envs(extra.iter().copied())
        .stdout(open())
        .stderr(open());
    let node = NodeProc(c.spawn().unwrap());
    let token = h.join("control.token");
    let end = Instant::now() + Duration::from_secs(60);
    while !h.join("node.sock").exists() || (control_token && !token.exists()) {
        assert!(Instant::now() < end, "no node");
        std::thread::sleep(Duration::from_millis(50));
    }
    node
}

fn wait_for(file: &Path, what: &str, ok: impl Fn(&str) -> bool) -> String {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(t) = std::fs::read_to_string(file) {
            if ok(&t) {
                return t;
            }
        }
        assert!(Instant::now() < end, "{what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn worktree_of(out: &str) -> PathBuf {
    PathBuf::from(out.trim().rsplit(" in ").next().unwrap())
}

/// Every file under the Agora folder except the store itself and the agents' own dumps of their environment (made on purpose).
fn files_with(h: &Path, values: &[&str]) -> Vec<String> {
    let mut hits = Vec::new();
    let mut stack = vec![h.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p == h.join("data/secrets")
                || name == "probe.txt"
                || name == "probe.tmp"
                || name == ".git"
            {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(b) = std::fs::read(&p) {
                for v in values {
                    if b.windows(v.len()).any(|w| w == v.as_bytes()) {
                        hits.push(format!("{} has {v}", p.display()));
                    }
                }
            }
        }
    }
    hits
}

/// The whole path with the real binary and every place the node's output could go: the node's own stdout and stderr (a service sends
/// them to the journal), every CLI reply, the agent's terminal as a client sees it, and every file in the Agora folder. Runs a first
/// start, a start of another kind, a crash and the node's restart of the agent, a restart of the node itself, and a refused start.
/// Neither fake value may appear in any of them. (A mutation that prints the value to the node's stderr makes this fail.)
#[test]
fn a_secret_is_in_no_output_of_the_node_the_cli_the_terminal_or_a_file() {
    if !agora_platform::PTY_SUPPORTED || !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    // Out of a confined agent's reach: a confined node refuses to start where agents may write (ADR-0082), and `/tmp` is such a
    // place. Short enough for the node's sockets in a deep checkout (#1040); declared before every node, so removed after them.
    let dir = short_socket::SocketDir::new(
        &PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("agora-secret-out-{}", std::process::id())),
        "secret-out",
    );
    let h = dir.path().to_path_buf();
    let _ = std::fs::remove_dir_all(&h);
    std::fs::create_dir_all(h.join("data")).unwrap();
    let repo = repo(&h);
    manifest(
        &h,
        "probe",
        "PROBE_TOKEN",
        "env > probe.tmp && mv probe.tmp probe.txt; echo agent-is-up; sleep 120",
    );
    manifest(
        &h,
        "other",
        "OTHER_TOKEN",
        "env > probe.tmp && mv probe.tmp probe.txt; sleep 120",
    );
    manifest(
        &h,
        "crash",
        "CRASH_TOKEN",
        "env > probe.tmp && mv probe.tmp probe.txt; echo run >> runs.txt; exit 1",
    );
    std::fs::write(
        h.join("data/policy"),
        "credentials.probe = PROBE_TOKEN\ncredentials.crash = CRASH_TOKEN\n",
    )
    .unwrap();
    for n in ["PROBE_TOKEN", "CRASH_TOKEN"] {
        let (code, out, err) = run(&h, &["node", "secret", "set", n], &format!("{VALUE}\n"));
        assert_eq!(code, 0, "{err}");
        assert!(!out.contains(VALUE) && !err.contains(VALUE));
    }
    let log = h.join("node.log");
    let mut seen_cli = String::new();
    // The node's own environment carries another kind's token (it is scrubbed, and must not be printed either).
    let node = start_node(&h, &repo, &log, &[("OTHER_TOKEN", OTHER)], false);
    let start = |kind: &str| {
        let (code, out, err) = run(
            &h,
            &["agent", "start", kind, "--project", "demo", "--worktree"],
            "",
        );
        assert_eq!(code, 0, "{err}");
        (out, err)
    };
    let (out, err) = start("probe");
    seen_cli += &format!("{out}{err}");
    let probe_wt = worktree_of(&out);
    let env = wait_for(&probe_wt.join("probe.txt"), "probe never reported", |_| {
        true
    });
    assert!(env.contains(&format!("PROBE_TOKEN={VALUE}")));
    let (out, err) = start("other");
    seen_cli += &format!("{out}{err}");
    let other = wait_for(
        &worktree_of(&out).join("probe.txt"),
        "other never reported",
        |_| true,
    );
    assert!(
        !other.contains("OTHER_TOKEN") && !other.contains(VALUE),
        "scrubbed and not given"
    );
    // A crash: the node starts the agent again, with the secret again.
    let (out, err) = start("crash");
    seen_cli += &format!("{out}{err}");
    wait_for(&worktree_of(&out).join("runs.txt"), "no restart", |t| {
        t.lines().count() >= 2
    });
    // The agent's terminal as a client sees it (the snapshot of its scrollback), in a real terminal.
    let mut cfg = PtyConfig::new(vec![
        AGORA.into(),
        "terminal".into(),
        "attach".into(),
        "agent-1".into(),
    ]);
    cfg.env.push(("AGORA_HOME".into(), h.display().to_string()));
    cfg.env.push(("AGORA_LANG".into(), "en".into()));
    let attach = spawn(&cfg).unwrap();
    let mut screen = String::new();
    let end = Instant::now() + Duration::from_secs(15);
    while !screen.contains("agent-is-up") {
        assert!(
            Instant::now() < end,
            "never saw the agent's terminal: {screen:?}"
        );
        if let Ok(chunk) = attach.output().recv_timeout(Duration::from_millis(100)) {
            screen.push_str(&String::from_utf8_lossy(&chunk));
        }
    }
    drop(attach);
    // The node itself restarts: its output continues in the same log.
    drop(node);
    let node = start_node(&h, &repo, &log, &[("OTHER_TOKEN", OTHER)], false);
    let (_, out, err) = run(&h, &["agent", "list"], "");
    seen_cli += &format!("{out}{err}");
    let (_, out, err) = run(&h, &["doctor"], "");
    seen_cli += &format!("{out}{err}");
    let (_, out, err) = run(&h, &["node", "secret", "list"], "");
    seen_cli += &format!("{out}{err}");
    std::thread::sleep(Duration::from_millis(500));
    drop(node);

    // An error path: a confined start whose profile reaches the store is refused, and neither the reply nor the node's output has it.
    let mut refused = String::new();
    if agora_platform::sandbox_supported() {
        std::fs::write(
            h.join("data/policy"),
            format!(
                "sandbox = worktree\nsandbox.read = {}\ncredentials.probe = PROBE_TOKEN\n",
                h.join("data").display()
            ),
        )
        .unwrap();
        let node = start_node(&h, &repo, &log, &[], agora_platform::CONTROL_TOKEN_REQUIRED);
        let (code, out, err) = run(
            &h,
            &["agent", "start", "probe", "--project", "demo", "--worktree"],
            "",
        );
        refused = format!("{out}{err}");
        // The refusal is the one under test, not some other failure.
        assert_ne!(code, 0, "{refused}");
        assert!(refused.contains("agent.sandbox_unavailable"), "{refused}");
        std::thread::sleep(Duration::from_millis(300));
        drop(node);
    }

    let node_log = std::fs::read_to_string(&log).unwrap();
    for (label, text) in [
        ("the node's stdout and stderr", node_log.as_str()),
        ("the CLI's replies", seen_cli.as_str()),
        ("the agent's terminal", screen.as_str()),
        ("the refused start's reply", refused.as_str()),
    ] {
        assert!(
            !text.contains(VALUE) && !text.contains(OTHER),
            "a secret is in {label}"
        );
    }
    assert_eq!(files_with(&h, &[VALUE, OTHER]), Vec::<String>::new());
    let _ = std::fs::remove_dir_all(&h);
}
