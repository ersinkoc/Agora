//! ADR-0084: `agora node secret set|list|remove` run end to end as real processes: the value comes from standard input and is never
//! echoed, listed, put in an argument, or written by the node into its output. The "agent" is a plain `sh` that dumps its environment.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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

/// The whole path with the real binary: store a secret, list it in the policy, run the node, start the agent; the agent has the value
/// and neither the node's output nor anything the node wrote does.
#[test]
fn a_node_passes_the_stored_secret_and_never_prints_or_keeps_it() {
    if !agora_platform::PTY_SUPPORTED || !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let h = home("node");
    let repo = h.join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "hi").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    std::fs::create_dir_all(h.join("manifests")).unwrap();
    std::fs::write(
        h.join("manifests/probe.toml"),
        "kind = \"probe\"\ndisplay_name = \"Probe\"\nversion = \"1\"\nmin_engine = 4\n[process]\nlaunch = [\"sh\", \"-c\", \"env > probe.tmp && mv probe.tmp probe.txt; sleep 60\"]\n[accounts]\ncredential_env = [\"PROBE_TOKEN\"]\n",
    )
    .unwrap();
    std::fs::write(h.join("data/policy"), "credentials.probe = PROBE_TOKEN\n").unwrap();
    let (code, _, err) = run(
        &h,
        &["node", "secret", "set", "PROBE_TOKEN"],
        &format!("{VALUE}\n"),
    );
    assert_eq!(code, 0, "{err}");

    let log = h.join("node.log");
    let node = NodeProc(
        Command::new(AGORA)
            .args(["node", "run", "--project"])
            .arg(&repo)
            .env("AGORA_HOME", &h)
            .stdout(std::fs::File::create(&log).unwrap())
            .stderr(std::fs::OpenOptions::new().append(true).open(&log).unwrap())
            .spawn()
            .unwrap(),
    );
    let end = Instant::now() + Duration::from_secs(60);
    while !h.join("node.sock").exists() {
        assert!(Instant::now() < end, "no node");
        std::thread::sleep(Duration::from_millis(50));
    }
    let (code, out, err) = run(
        &h,
        &["agent", "start", "probe", "--project", "demo", "--worktree"],
        "",
    );
    assert_eq!(code, 0, "{err}");
    assert!(!out.contains(VALUE) && !err.contains(VALUE));
    let worktree = PathBuf::from(out.trim().rsplit(" in ").next().unwrap());
    let end = Instant::now() + Duration::from_secs(15);
    while !worktree.join("probe.txt").exists() {
        assert!(Instant::now() < end, "the agent never reported");
        std::thread::sleep(Duration::from_millis(20));
    }
    let env = std::fs::read_to_string(worktree.join("probe.txt")).unwrap();
    assert!(env.contains(&format!("PROBE_TOKEN={VALUE}")));
    // `agora doctor`: the name, a boolean, never the value.
    let (_, out, err) = run(&h, &["doctor"], "");
    assert!(
        out.contains("PROBE_TOKEN") || out.contains("probe"),
        "{out}"
    );
    assert!(!out.contains(VALUE) && !err.contains(VALUE));
    std::thread::sleep(Duration::from_millis(500));
    drop(node);
    let node_log = std::fs::read_to_string(&log).unwrap();
    assert!(!node_log.contains(VALUE), "the node's output has the value");
    for dir in [h.join("data"), h.join("hub")] {
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p == h.join("data/secrets") {
                    continue;
                }
                if p.is_dir() {
                    stack.push(p);
                } else if let Ok(b) = std::fs::read(&p) {
                    assert!(
                        !b.windows(VALUE.len()).any(|w| w == VALUE.as_bytes()),
                        "the value is in {}",
                        p.display()
                    );
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&h);
}
