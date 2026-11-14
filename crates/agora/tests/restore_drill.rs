//! The multi-machine backup and restore drill (docs/design/backup-restore.md; architecture review R8), with real processes: a hub,
//! two enrolled machines with an agent each, a backup, more work after it, the hub lost and brought back from the older backup, and
//! the machines coming back by themselves. It checks the recovery outcome at each hop and prints how long recovery took.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use agora_hub::projection::View;
use agora_hub::store::SqliteStore;

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn agora(home: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(AGORA);
    c.args(args).env("AGORA_HOME", home).env("AGORA_LANG", "en");
    c
}

fn run(home: &Path, args: &[&str]) -> String {
    let out = agora(home, args).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "agora {args:?}: {text}");
    text
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
    assert!(ok);
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agora-drill-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Starts the hub and returns it with the address it listens on (its first line says it).
fn hub(home: &Path, hub_dir: &Path, listen: &str) -> (Proc, String) {
    let mut child = agora(home, &["hub", "run", "--listen", listen, "--hub-dir"])
        .arg(hub_dir)
        // A revoked certificate ends an open connection at the next recheck (10 s by default): shorter here.
        .env("AGORA_RECHECK_MS", "200")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let addr = line
        .split("running on ")
        .nth(1)
        .and_then(|r| r.split(". Its").next())
        .expect(&line)
        .trim()
        .to_string();
    (Proc(child), addr)
}

/// The hub's view, read from its file (while it runs, or after it stopped).
fn view(hub_dir: &Path) -> View {
    View::rebuild(&SqliteStore::open(&hub_dir.join("hub.db")).unwrap())
        .unwrap()
        .0
}

fn wait_view(hub_dir: &Path, what: &str, within: Duration, ok: impl Fn(&View) -> bool) -> Duration {
    let start = Instant::now();
    loop {
        let v = view(hub_dir);
        if ok(&v) {
            return start.elapsed();
        }
        assert!(
            start.elapsed() < within,
            "{what}: never happened. The hub sees {:?} and agents {:?}",
            v.nodes
                .iter()
                .map(|(k, n)| (k.clone(), n.online))
                .collect::<Vec<_>>(),
            v.agents
                .iter()
                .map(|(k, a)| (k.clone(), a.running))
                .collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// An enrolled machine running a node with one project and a shell agent kind.
fn machine(root: &Path, hub_dir: &Path, port: &str, id: &str) -> (PathBuf, Proc) {
    let home = root.join(id);
    std::fs::create_dir_all(home.join("manifests")).unwrap();
    let code_text = run(
        &home,
        &["hub", "code", "--hub-dir", hub_dir.to_str().unwrap()],
    );
    let code = code_text
        .split("Enrollment code: ")
        .nth(1)
        .and_then(|r| r.split(' ').next())
        .expect(&code_text)
        .to_string();
    run(
        &home,
        &[
            "node",
            "enroll",
            "--hub",
            &format!("wss://localhost:{port}"),
            "--ca",
            hub_dir.join("ca.pem").to_str().unwrap(),
            "--code",
            &code,
            "--id",
            id,
        ],
    );
    let repo = root.join(format!("{id}-repo"));
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), id).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    std::fs::write(
        home.join("manifests/shell.toml"),
        "kind = \"shell\"\ndisplay_name = \"Shell\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n",
    )
    .unwrap();
    let node = agora(&home, &["node", "run", "--project"])
        .arg(&repo)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let sock = home.join("node.sock");
    let end = Instant::now() + Duration::from_secs(15);
    while !sock.exists() {
        assert!(Instant::now() < end, "{id}: the node did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    (home, Proc(node))
}

#[test]
fn the_hub_comes_back_from_an_older_backup_and_every_machine_brings_its_present() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let root = tmp("restore");
    let (hub_home, hub_dir) = (root.join("hub-home"), root.join("hub"));
    std::fs::create_dir_all(&hub_home).unwrap();
    let (hub1, addr) = hub(&hub_home, &hub_dir, "127.0.0.1:0");
    let port = addr.rsplit(':').next().unwrap().to_string();

    // Two machines, each with an agent.
    let (alpha, _alpha_node) = machine(&root, &hub_dir, &port, "alpha");
    let (beta, _beta_node) = machine(&root, &hub_dir, &port, "beta");
    for home in [&alpha, &beta] {
        run(
            home,
            &[
                "agent",
                "start",
                "shell",
                "--project",
                &format!("{}-repo", home.file_name().unwrap().to_string_lossy()),
            ],
        );
    }
    wait_view(
        &hub_dir,
        "both machines and their agents",
        Duration::from_secs(30),
        |v| {
            ["alpha", "beta"]
                .iter()
                .all(|n| v.nodes.get(*n).is_some_and(|n| n.online))
                && v.agents.get("alpha/agent-1").is_some_and(|a| a.running)
                && v.agents.get("beta/agent-1").is_some_and(|a| a.running)
        },
    );

    // The backup, taken while the hub runs.
    let backup = root.join("backup.db");
    run(
        &hub_home,
        &[
            "hub",
            "backup",
            backup.to_str().unwrap(),
            "--hub-dir",
            hub_dir.to_str().unwrap(),
        ],
    );

    // A third machine is enrolled after the backup: the restored hub has never heard of its certificate.
    let (gamma, gamma_node) = machine(&root, &hub_dir, &port, "gamma");
    wait_view(&hub_dir, "gamma online", Duration::from_secs(30), |v| {
        v.nodes.get("gamma").is_some_and(|n| n.online)
    });

    // Work after the backup: a second agent on alpha, and beta's agent ends.
    run(
        &alpha,
        &["agent", "start", "shell", "--project", "alpha-repo"],
    );
    run(&beta, &["agent", "stop", "agent-1"]);
    wait_view(
        &hub_dir,
        "the work after the backup",
        Duration::from_secs(30),
        |v| {
            v.agents.get("alpha/agent-2").is_some_and(|a| a.running)
                && v.agents.get("beta/agent-1").is_some_and(|a| !a.running)
        },
    );

    // Disaster: the hub is lost and comes back from the older backup.
    drop(hub1);
    run(
        &hub_home,
        &[
            "hub",
            "restore",
            backup.to_str().unwrap(),
            "--hub-dir",
            hub_dir.to_str().unwrap(),
        ],
    );
    let restored = view(&hub_dir);
    assert!(
        !restored.agents.contains_key("alpha/agent-2"),
        "the restored file is the backup: it has not seen the second agent"
    );
    assert!(
        restored
            .agents
            .get("beta/agent-1")
            .is_some_and(|a| a.running),
        "nor beta's agent ending"
    );

    // The hub starts again on its old address; the machines reconnect by themselves and bring what they still hold.
    let restarted = Instant::now();
    let (_hub2, _) = hub(&hub_home, &hub_dir, &format!("127.0.0.1:{port}"));
    let recovered = wait_view(
        &hub_dir,
        "the machines' present after the restore",
        Duration::from_secs(60),
        |v| {
            ["alpha", "beta"]
                .iter()
                .all(|n| v.nodes.get(*n).is_some_and(|n| n.online))
                && v.agents.get("alpha/agent-1").is_some_and(|a| a.running)
                && v.agents.get("alpha/agent-2").is_some_and(|a| a.running)
                && v.agents.get("beta/agent-1").is_some_and(|a| !a.running)
        },
    );
    // The machines still held every event after the backup, so nothing was lost: no history range is recorded as missing.
    let lost = SqliteStore::open(&hub_dir.join("hub.db"))
        .unwrap()
        .resyncs()
        .unwrap();
    assert!(
        lost.is_empty(),
        "history recorded as lost although the machines still held it: {lost:?}"
    );
    let whole = restarted.elapsed();
    // The recovery target the maintainer approved (backup-restore.md): the present is whole within 5 s of the hub starting.
    assert!(
        whole <= Duration::from_secs(5),
        "the restored hub's present took {} ms, over the 5 s recovery target",
        whole.as_millis()
    );

    // The machine enrolled after the backup is refused: its certificate is unknown to the restored hub (it fails closed). It has to
    // be enrolled again, and then it connects.
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        !view(&hub_dir).nodes.get("gamma").is_some_and(|n| n.online),
        "a certificate the restored hub never issued was accepted"
    );
    drop(gamma_node);
    let code_text = run(
        &gamma,
        &["hub", "code", "--hub-dir", hub_dir.to_str().unwrap()],
    );
    let code = code_text
        .split("Enrollment code: ")
        .nth(1)
        .and_then(|r| r.split(' ').next())
        .expect(&code_text)
        .to_string();
    run(
        &gamma,
        &[
            "node",
            "enroll",
            "--hub",
            &format!("wss://localhost:{port}"),
            "--ca",
            hub_dir.join("ca.pem").to_str().unwrap(),
            "--code",
            &code,
            "--id",
            "gamma",
        ],
    );
    let node = agora(&gamma, &["node", "run", "--project"])
        .arg(root.join("gamma-repo"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _gamma_again = Proc(node);
    wait_view(
        &hub_dir,
        "gamma back after enrolling again",
        Duration::from_secs(30),
        |v| v.nodes.get("gamma").is_some_and(|n| n.online),
    );
    eprintln!(
        "restore drill: the hub's present was whole {} ms after it started from the backup ({} ms waiting on the machines); lost history ranges: none; the machine enrolled after the backup was refused until it enrolled again",
        whole.as_millis(),
        recovered.as_millis()
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The certificate of a machine that is connected is revoked (it enrolled again under the same name, which replaces the old identity):
/// the open connection ends, the old node is refused when it redials, and the new one connects.
#[test]
fn revoking_a_connected_machine_ends_its_connection_and_keeps_it_out() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let root = tmp("revoke");
    let (hub_home, hub_dir) = (root.join("hub-home"), root.join("hub"));
    std::fs::create_dir_all(&hub_home).unwrap();
    let (_hub, addr) = hub(&hub_home, &hub_dir, "127.0.0.1:0");
    let port = addr.rsplit(':').next().unwrap().to_string();
    let (_old_home, old_node) = machine(&root, &hub_dir, &port, "alpha");
    wait_view(&hub_dir, "alpha online", Duration::from_secs(30), |v| {
        v.nodes.get("alpha").is_some_and(|n| n.online)
    });

    // Another machine enrolls as `alpha`: the first one's certificate is revoked while its connection is open.
    let new_home = root.join("alpha-new");
    std::fs::create_dir_all(new_home.join("manifests")).unwrap();
    let code_text = run(
        &new_home,
        &["hub", "code", "--hub-dir", hub_dir.to_str().unwrap()],
    );
    let code = code_text
        .split("Enrollment code: ")
        .nth(1)
        .and_then(|r| r.split(' ').next())
        .expect(&code_text)
        .to_string();
    let revoked = Instant::now();
    run(
        &new_home,
        &[
            "node",
            "enroll",
            "--hub",
            &format!("wss://localhost:{port}"),
            "--ca",
            hub_dir.join("ca.pem").to_str().unwrap(),
            "--code",
            &code,
            "--id",
            "alpha",
        ],
    );
    let ended = wait_view(
        &hub_dir,
        "the revoked connection ends",
        Duration::from_secs(30),
        |v| v.nodes.get("alpha").is_some_and(|n| !n.online),
    );
    // It keeps redialing with the revoked certificate and stays out.
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        !view(&hub_dir).nodes.get("alpha").is_some_and(|n| n.online),
        "the old machine came back with a revoked certificate"
    );
    drop(old_node);

    // The new machine, with the new certificate, is let in.
    let repo = root.join("alpha-new-repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "new").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let log = std::fs::File::create(root.join("alpha-new.log")).unwrap();
    let node = agora(&new_home, &["node", "run", "--project"])
        .arg(&repo)
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .unwrap();
    let _new_node = Proc(node);
    let sock = new_home.join("node.sock");
    let end = Instant::now() + Duration::from_secs(15);
    while !sock.exists() {
        assert!(
            Instant::now() < end,
            "the new alpha's node did not start: {}",
            std::fs::read_to_string(root.join("alpha-new.log")).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let end = Instant::now() + Duration::from_secs(30);
    while !view(&hub_dir).nodes.get("alpha").is_some_and(|n| n.online) {
        assert!(
            Instant::now() < end,
            "the new alpha never came online; its node said: {}",
            std::fs::read_to_string(root.join("alpha-new.log")).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    eprintln!(
        "revocation drill: the open connection ended within {} ms of the old certificate's revocation; the old node stayed out; the new one connected",
        ended.as_millis().max(revoked.elapsed().as_millis().min(1))
    );
    let _ = std::fs::remove_dir_all(&root);
}
