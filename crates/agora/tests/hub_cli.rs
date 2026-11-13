//! Three real `agora` processes: a hub, an enrollment, and a node that connects by itself and shows up with its
//! agent in the hub's database. The Phase 3 flow from the command line (agents are plain `sh`).
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
    let d = std::env::temp_dir().join(format!("agora-hubcli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn a_machine_enrolls_connects_and_its_agent_shows_up_in_the_hub() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let root = tmp("flow");
    let (hub_dir, node_home) = (root.join("hub"), root.join("node"));
    std::fs::create_dir_all(&node_home).unwrap();

    // The hub.
    let mut hub = agora(
        &node_home,
        &["hub", "run", "--listen", "127.0.0.1:0", "--hub-dir"],
    )
    .arg(&hub_dir)
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    let mut line = String::new();
    // Kept open: the hub prints more after its first line.
    let mut out = BufReader::new(hub.stdout.take().unwrap());
    out.read_line(&mut line).unwrap();
    let _hub = Proc(hub);
    let addr = line
        .split("running on ")
        .nth(1)
        .and_then(|r| r.split(". Its").next())
        .expect(&line)
        .trim()
        .to_string();
    let port = addr.rsplit(':').next().unwrap();

    // A one-time code, then enroll this "machine" (its own AGORA_HOME).
    let out = agora(&node_home, &["hub", "code", "--hub-dir"])
        .arg(&hub_dir)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let code = text
        .split("Enrollment code: ")
        .nth(1)
        .and_then(|r| r.split(' ').next())
        .expect(&text)
        .to_string();
    let out = agora(
        &node_home,
        &[
            "node",
            "enroll",
            "--hub",
            &format!("wss://localhost:{port}"),
            "--ca",
        ],
    )
    .arg(hub_dir.join("ca.pem"))
    .args(["--code", &code, "--id", "laptop"])
    .output()
    .unwrap();
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(node_home.join("link/cert.pem").exists());

    // The code was single use: a second machine with the same code is refused.
    let other = tmp("other-home");
    let out = agora(
        &other,
        &[
            "node",
            "enroll",
            "--hub",
            &format!("wss://localhost:{port}"),
            "--ca",
        ],
    )
    .arg(hub_dir.join("ca.pem"))
    .args(["--code", &code, "--id", "second"])
    .output()
    .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("refused to enroll"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The node: a project, a shell agent, and it connects to the hub on its own.
    let repo = root.join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "hi").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    std::fs::create_dir_all(node_home.join("manifests")).unwrap();
    std::fs::write(
        node_home.join("manifests/shell.toml"),
        "kind = \"shell\"\ndisplay_name = \"Shell\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n",
    )
    .unwrap();
    let node = agora(&node_home, &["node", "run", "--project"])
        .arg(&repo)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _node = Proc(node);
    let sock = node_home.join("node.sock");
    let end = Instant::now() + Duration::from_secs(15);
    while !sock.exists() {
        assert!(Instant::now() < end, "node did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = agora(
        &node_home,
        &["agent", "start", "shell", "--project", "demo"],
    )
    .output()
    .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The hub's own database (read here while the hub runs) shows the machine and its agent.
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let store = SqliteStore::open(&hub_dir.join("hub.db")).unwrap();
        let (view, _) = View::rebuild(&store).unwrap();
        if view.nodes.get("laptop").is_some_and(|n| n.online)
            && view.agents.contains_key("laptop/agent-1")
        {
            break;
        }
        assert!(
            Instant::now() < end,
            "the hub never saw the node and its agent: {view:?}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn the_hub_database_can_be_backed_up_pruned_and_restored_from_the_command_line() {
    let home = tmp("backup");
    // `hub code` creates the hub folder and database.
    assert!(agora(&home, &["hub", "code"])
        .output()
        .unwrap()
        .status
        .success());
    let db = home.join("hub/hub.db");
    assert!(db.exists());

    let out = agora(&home, &["hub", "backup", "--keep", "2"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    for _ in 0..3 {
        std::thread::sleep(Duration::from_millis(5));
        assert!(agora(&home, &["hub", "backup", "--keep", "2"])
            .output()
            .unwrap()
            .status
            .success());
    }
    let kept = std::fs::read_dir(home.join("hub/backups")).unwrap().count();
    assert_eq!(kept, 2, "retention keeps the newest two");

    // An explicit file never overwrites.
    let file = home.join("manual.db");
    let file_s = file.to_str().unwrap();
    assert!(agora(&home, &["hub", "backup", file_s])
        .output()
        .unwrap()
        .status
        .success());
    assert!(!agora(&home, &["hub", "backup", file_s])
        .output()
        .unwrap()
        .status
        .success());

    // Restore replaces the database and keeps the old one.
    assert!(agora(&home, &["hub", "restore", file_s])
        .output()
        .unwrap()
        .status
        .success());
    assert!(home.join("hub/hub.db.before-restore").exists());
    // A file that is not a hub database is refused and changes nothing.
    std::fs::write(home.join("junk.db"), "not sqlite").unwrap();
    assert!(!agora(
        &home,
        &["hub", "restore", home.join("junk.db").to_str().unwrap()]
    )
    .output()
    .unwrap()
    .status
    .success());
    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn notification_targets_are_added_listed_and_tested_from_the_command_line() {
    use std::io::{Read, Write};
    let root = tmp("notify");
    let hub = root.join("hub");
    let hub = hub.to_str().unwrap();
    let run = |args: &[&str]| agora(&root, args).output().unwrap();

    let none = run(&["hub", "notify", "--hub-dir", hub]);
    assert!(String::from_utf8_lossy(&none.stdout).contains("No notification targets yet"));
    let bad = run(&["hub", "notify", "--webhook", "ftp://x", "--hub-dir", hub]);
    assert_eq!(bad.status.code(), Some(2));

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/hook");
    let added = run(&["hub", "notify", "--webhook", &url, "--hub-dir", hub]);
    assert!(added.status.success(), "{added:?}");
    let listed = run(&["hub", "notify", "--hub-dir", hub]);
    assert!(String::from_utf8_lossy(&listed.stdout).contains(&format!("webhook 127.0.0.1:{port}")));

    let server = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut got = vec![0u8; 2048];
        let n = s.read(&mut got).unwrap();
        s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
        String::from_utf8_lossy(&got[..n]).to_string()
    });
    let tested = run(&["hub", "notify", "--test", "--hub-dir", hub]);
    assert!(tested.status.success(), "{tested:?}");
    assert!(server.join().unwrap().contains("Agora notifications work."));
}

#[test]
fn the_price_table_is_refreshed_only_on_request_and_a_bad_answer_changes_nothing() {
    use std::io::{Read, Write};
    let root = tmp("prices");
    let hub = root.join("hub");
    let serve = |body: &'static str| {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut got = [0u8; 512];
            let _ = s.read(&mut got).unwrap();
            s.write_all(format!("HTTP/1.0 200 OK\r\n\r\n{body}").as_bytes())
                .unwrap();
        });
        format!("http://127.0.0.1:{port}/api.json")
    };
    let update = |url: &str| {
        agora(
            &root,
            &[
                "hub",
                "prices",
                "update",
                "--url",
                url,
                "--hub-dir",
                hub.to_str().unwrap(),
            ],
        )
        .output()
        .unwrap()
    };
    let bad = update(&serve("{\"p\":{\"models\":{}}}"));
    assert_eq!(bad.status.code(), Some(1));
    assert!(!hub.join("prices-catalog.json").exists());
    let ok = update(&serve(
        "{\"p\":{\"models\":{\"m\":{\"cost\":{\"input\":1,\"output\":2}}}}}",
    ));
    assert!(ok.status.success(), "{ok:?}");
    assert!(String::from_utf8_lossy(&ok.stdout).contains("1 models"));
    assert!(hub.join("prices-catalog.json").exists());
}

// The stand-in for ssh is a shell script with the executable bit.
#[cfg(unix)]
#[test]
fn a_machine_added_over_ssh_gets_a_tunnel_the_hub_keeps_open() {
    let root = tmp("ssh");
    let hub_dir = root.join("hub");
    let fake = root.join("fake-ssh");
    let log = root.join("args");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\necho \"$@\" >> {}\nexec sleep 60\n",
            log.display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let run = |args: &[&str]| {
        agora(&root, args)
            .arg("--hub-dir")
            .arg(&hub_dir)
            .output()
            .unwrap()
    };

    assert!(String::from_utf8_lossy(&run(&["hub", "ssh"]).stdout).contains("No SSH machines yet"));
    assert_eq!(
        run(&["hub", "ssh", "add", "box", "-oProxyCommand=x"])
            .status
            .code(),
        Some(2)
    );
    let added = run(&["hub", "ssh", "add", "box", "user@box.example"]);
    assert!(added.status.success(), "{added:?}");
    let said = String::from_utf8_lossy(&added.stdout).into_owned();
    assert!(
        said.contains("wss://localhost:17443") && said.contains("--id box"),
        "{said}"
    );
    assert_eq!(
        run(&["hub", "ssh", "add", "box", "other"]).status.code(),
        Some(2),
        "names are unique"
    );

    let mut hub = agora(
        &root,
        &["hub", "run", "--listen", "127.0.0.1:0", "--hub-dir"],
    )
    .arg(&hub_dir)
    .env("AGORA_SSH", &fake)
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    let mut line = String::new();
    // Kept open: the hub prints more after its first line, and a closed pipe would end it.
    let mut out = BufReader::new(hub.stdout.take().unwrap());
    out.read_line(&mut line).unwrap();
    let _hub = Proc(hub);
    let t0 = Instant::now();
    let args = loop {
        if let Ok(a) = std::fs::read_to_string(&log) {
            if !a.is_empty() {
                break a;
            }
        }
        assert!(
            t0.elapsed() < Duration::from_secs(15),
            "the tunnel was never started"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(
        args.contains("BatchMode=yes") && args.contains("StrictHostKeyChecking=yes"),
        "{args}"
    );
    assert!(
        args.contains("-R 127.0.0.1:17443:127.0.0.1:") && args.contains("-- user@box.example"),
        "{args}"
    );
    let t0 = Instant::now();
    loop {
        let listed = String::from_utf8_lossy(&run(&["hub", "ssh"]).stdout).into_owned();
        if listed.contains("box user@box.example 17443 up") {
            break;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(15),
            "never reported up: {listed}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(unix)]
#[test]
fn hub_ssh_install_walks_the_four_steps_on_a_machine_and_stops_with_the_failing_step() {
    let root = tmp("ssh-install");
    let hub_dir = root.join("hub");
    let fake = root.join("fake-ssh");
    let log = root.join("log");
    // The stand-in answers like a fresh Linux box with systemd, records each command and what it was given on stdin.
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nfor last; do :; done\nprintf '%s\\n' \"$last\" >> {log}\ncat >> {log}.stdin\ncase \"$last\" in\n  'uname -sm') echo 'Linux x86_64';;\n  'uname -s') echo Linux;;\n  *--version*) exit 1;;\n  *'node enroll'*) [ -f {root}/refuse ] && {{ echo refused >&2; exit 1; }};;\nesac\nexit 0\n",
            log = log.display(),
            root = root.display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let run = |args: &[&str]| {
        agora(&root, args)
            .arg("--hub-dir")
            .arg(&hub_dir)
            .env("AGORA_SSH", &fake)
            .output()
            .unwrap()
    };
    assert_eq!(
        run(&["hub", "ssh", "install", "box", "--yes"])
            .status
            .code(),
        Some(2),
        "unknown machine"
    );
    assert!(run(&["hub", "ssh", "add", "box", "user@box.example"])
        .status
        .success());

    // Without a pinned key it refuses to fetch anything.
    let refused = run(&[
        "hub",
        "ssh",
        "install",
        "box",
        "--yes",
        "--release-url",
        "https://example.test/dl",
    ]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("binary"),
        "{refused:?}"
    );
    assert!(!std::fs::read_to_string(&log).unwrap().contains("curl"));

    let ok = run(&[
        "hub",
        "ssh",
        "install",
        "box",
        "--yes",
        "--unsigned",
        "--release-url",
        "https://example.test/dl",
    ]);
    assert!(ok.status.success(), "{ok:?}");
    let said = String::from_utf8_lossy(&ok.stdout).into_owned();
    assert!(said.contains("4/4") && said.contains("systemd"), "{said}");
    let commands = std::fs::read_to_string(&log).unwrap();
    assert!(commands.contains("https://example.test/dl/v"), "{commands}");
    assert!(
        commands.contains("node enroll --hub wss://localhost:17443 --ca - --code "),
        "{commands}"
    );
    // The CA went over stdin, not onto the command line.
    assert!(std::fs::read_to_string(format!("{}.stdin", log.display()))
        .unwrap()
        .contains("BEGIN CERTIFICATE"));

    // A refused enrollment stops there and says what was done.
    std::fs::write(root.join("refuse"), "").unwrap();
    let stopped = run(&[
        "hub",
        "ssh",
        "install",
        "box",
        "--yes",
        "--unsigned",
        "--release-url",
        "https://example.test/dl",
    ]);
    assert_eq!(stopped.status.code(), Some(1));
    let err = String::from_utf8_lossy(&stopped.stderr).into_owned();
    assert!(
        err.contains("enroll") && err.contains("refused") && err.contains("check, binary"),
        "{err}"
    );
}

#[test]
fn half_a_ca_is_an_error_not_a_reason_to_make_a_new_one() {
    let root = tmp("half-ca");
    let hub_dir = root.join("hub");
    let first = agora(&root, &["hub", "code"])
        .arg("--hub-dir")
        .arg(&hub_dir)
        .output()
        .unwrap();
    assert!(first.status.success(), "{first:?}");
    let cert_before = std::fs::read_to_string(hub_dir.join("ca.pem")).unwrap();
    // The key goes missing (a restore that forgot it, a permission slip).
    std::fs::remove_file(hub_dir.join("ca.key")).unwrap();
    let again = agora(&root, &["hub", "code"])
        .arg("--hub-dir")
        .arg(&hub_dir)
        .output()
        .unwrap();
    assert!(!again.status.success(), "{again:?}");
    assert_eq!(
        std::fs::read_to_string(hub_dir.join("ca.pem")).unwrap(),
        cert_before,
        "the certificate must not be replaced"
    );
    assert!(!hub_dir.join("ca.key").exists(), "no new key either");
}

#[test]
fn team_members_are_added_listed_changed_and_removed_from_the_command_line() {
    let root = tmp("users");
    let run = |args: &[&str]| {
        let o = agora(&root, args)
            .arg("--hub-dir")
            .arg(&root)
            .output()
            .unwrap();
        (
            o.status.success(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    let (ok, out, _) = run(&["hub", "user", "list"]);
    assert!(ok && out.contains("No team members"), "{out}");
    let (ok, out, err) = run(&["hub", "user", "add", "Ayse", "--role", "viewer"]);
    assert!(
        ok && out.contains("ayse") && out.contains("viewer") && out.contains('-'),
        "{out}{err}"
    );
    let (ok, _, _) = run(&["hub", "user", "add", "Bad", "--role", "root"]);
    assert!(!ok);
    let (ok, out, _) = run(&["hub", "user", "list"]);
    assert!(
        ok && out.contains("ayse") && out.contains("viewer"),
        "{out}"
    );
    let (ok, _, _) = run(&["hub", "user", "role", "ayse", "member"]);
    assert!(ok);
    // A person sees and works in the projects they were granted, `NODE/PROJECT`, and nothing else.
    let (ok, _, _) = run(&["hub", "user", "grant", "ayse", "laptop/web"]);
    assert!(ok);
    let (ok, _, _) = run(&["hub", "user", "grant", "ayse", "no-slash"]);
    assert!(!ok);
    let (ok, _, _) = run(&["hub", "user", "grant", "nobody", "laptop/web"]);
    assert!(!ok);
    let (ok, out, _) = run(&["hub", "user", "list"]);
    assert!(ok && out.contains("works in laptop/web"), "{out}");
    let (ok, _, _) = run(&["hub", "user", "revoke", "ayse", "laptop/web"]);
    assert!(ok);
    let (ok, out, _) = run(&["hub", "user", "list"]);
    assert!(ok && !out.contains("laptop/web"), "{out}");
    let (ok, out, _) = run(&["hub", "user", "login", "ayse"]);
    assert!(ok && out.contains("Sign-in code"), "{out}");
    let (ok, _, err) = run(&["hub", "user", "remove", "nobody"]);
    assert!(!ok && err.contains("nobody"), "{err}");
    let (ok, _, _) = run(&["hub", "user", "remove", "ayse"]);
    assert!(ok);
    // Every change to the team is in the audit trail, one JSON object per line, oldest first.
    let (ok, out, err) = run(&["hub", "audit", "export"]);
    assert!(ok, "{err}");
    let actions: Vec<String> = out
        .lines()
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            format!(
                "{}:{}",
                v["action"].as_str().unwrap(),
                v["node"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(
        actions,
        [
            "user.add:ayse",
            "user.role:ayse",
            "user.grant:ayse laptop/web",
            "user.revoke:ayse laptop/web",
            "user.remove:ayse"
        ],
        "{out}"
    );
    let (ok, out, _) = run(&["hub", "audit", "export", "--who", "nobody"]);
    assert!(ok && out.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_hub_prints_its_ca_fingerprint_for_the_desktop_apps_first_use() {
    let root = tmp("fingerprint");
    let run = |args: &[&str]| {
        let o = agora(&root, args)
            .arg("--hub-dir")
            .arg(&root)
            .output()
            .unwrap();
        (
            o.status.success(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
        )
    };
    // No CA yet: a clear failure, not a made-up value.
    assert!(!run(&["hub", "fingerprint"]).0);
    let (cert, key) = (root.join("ca.pem"), root.join("ca.key"));
    let ca = agora_hub::ca::Ca::generate("hub CA").unwrap();
    std::fs::write(&cert, ca.cert_pem()).unwrap();
    std::fs::write(&key, ca.key_pem()).unwrap();
    let (ok, out) = run(&["hub", "fingerprint"]);
    assert!(ok, "{out}");
    let want = agora_desktop_tunnel::fingerprint(ca.cert_pem()).unwrap();
    assert!(out.contains(&want), "{out}");
    let _ = std::fs::remove_dir_all(root);
}

/// The first `XXXX-XXXX` code in a command's output (Fluent may wrap it in isolation marks).
fn code_in(text: &str) -> String {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .find(|w| w.len() == 9 && w.as_bytes()[4] == b'-')
        .expect(text)
        .to_string()
}

#[test]
fn every_capability_a_node_advertises_is_one_the_real_hub_agrees_on() {
    // The real binaries on both ends: the hub's supported list (`hubcli.rs`) and the node's advertised list (`uplink_client.rs`)
    // must both name a capability, or the hub refuses every command that needs it (#378: `directory.list` was missing on the hub
    // side, and no test crossed the two lists). One command per capability goes through the hub; the node may refuse it for its
    // own reasons (no signature, no project), but the hub must not answer `protocol.capability_missing`.
    use agora_protocol::v1::{self, command_request::Command as Cmd};
    use agora_tui::client::{sign_in, Hub, Session};
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let root = tmp("capabilities");
    let (hub_dir, node_home) = (root.join("hub"), root.join("node"));
    std::fs::create_dir_all(&node_home).unwrap();
    let mut hub = agora(
        &node_home,
        &["hub", "run", "--listen", "127.0.0.1:0", "--hub-dir"],
    )
    .arg(&hub_dir)
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    let mut line = String::new();
    let mut out = BufReader::new(hub.stdout.take().unwrap());
    out.read_line(&mut line).unwrap();
    let _hub = Proc(hub);
    let addr = line
        .split("running on ")
        .nth(1)
        .and_then(|r| r.split(". Its").next())
        .expect(&line)
        .trim()
        .to_string();
    let port = addr.rsplit(':').next().unwrap().to_string();
    let text = agora(&node_home, &["hub", "code", "--hub-dir"])
        .arg(&hub_dir)
        .output()
        .unwrap();
    let code = code_in(&String::from_utf8_lossy(&text.stdout));
    let enrolled = agora(
        &node_home,
        &[
            "node",
            "enroll",
            "--hub",
            &format!("wss://localhost:{port}"),
            "--ca",
        ],
    )
    .arg(hub_dir.join("ca.pem"))
    .args(["--code", &code, "--id", "laptop"])
    .output()
    .unwrap();
    assert!(
        enrolled.status.success(),
        "{}",
        String::from_utf8_lossy(&enrolled.stderr)
    );
    let node = agora(&node_home, &["node", "run"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _node = Proc(node);

    let login = agora(&node_home, &["hub", "login", "--hub-dir"])
        .arg(&hub_dir)
        .output()
        .unwrap();
    let target = Hub {
        addr: format!("127.0.0.1:{port}"),
        ca_pem: std::fs::read_to_string(hub_dir.join("ca.pem")).unwrap(),
    };
    let token = sign_in(&target, &code_in(&String::from_utf8_lossy(&login.stdout))).unwrap();
    let mut session = Session::connect(&target, &token).unwrap();
    let wait = Duration::from_secs(30);
    for (capability, command) in [
        (
            "workspace.directory",
            Cmd::DirectoryInspect(v1::DirectoryInspect::default()),
        ),
        (
            "workspace.directory",
            Cmd::AgentLaunch(v1::AgentLaunch::default()),
        ),
        (
            "workspace.directory",
            Cmd::WorkspaceQuery(v1::WorkspaceQuery::default()),
        ),
        (
            "directory.list",
            Cmd::DirectoryList(v1::DirectoryList::default()),
        ),
        // `board.join` is not here: only the hub sends it, never a client. `hubcli`'s
        // `the_hub_agrees_on_every_capability_a_node_offers` covers it and every other capability a node offers.
    ] {
        // Until the node has dialed in, the hub does not know it (`node.unknown`) or has it offline.
        let end = Instant::now() + Duration::from_secs(60);
        let answer = loop {
            let answer = session.command("laptop", command.clone(), wait).err();
            let waiting = matches!(answer.as_deref(), Some("node.unknown" | "node.offline"));
            if !waiting || Instant::now() > end {
                break answer;
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        assert!(
            !matches!(answer.as_deref(), Some("node.unknown" | "node.offline")),
            "{capability}: the node never came online: {answer:?}"
        );
        // Anything else is the node's own answer (no signature, nothing to join), so the hub routed it.
        assert_ne!(
            answer.as_deref(),
            Some("protocol.capability_missing"),
            "{capability}: the hub and the node do not agree on it"
        );
    }
}

#[test]
fn the_screen_password_is_typed_twice_unseen_and_never_read_from_a_pipe() {
    use agora_hub::password::ScreenPassword;
    let root = tmp("password");
    let run = |args: &[&str]| {
        let o = agora(&root, args)
            .arg("--hub-dir")
            .arg(&root)
            .stdin(Stdio::piped())
            .output()
            .unwrap();
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
        )
    };
    let (code, out) = run(&["password", "status"]);
    assert_eq!(code, Some(0));
    assert!(out.contains("No screen password"), "{out}");
    // Not a terminal: refused, nothing set.
    assert_eq!(run(&["password", "set"]).0, Some(2));
    assert!(!ScreenPassword::new(&SqliteStore::open(&root.join("hub.db")).unwrap()).is_set());

    // A live session, to see it end when the password changes.
    let store = SqliteStore::open(&root.join("hub.db")).unwrap();
    let signin = agora_hub::signin::SignIn::new(
        &store,
        std::sync::Arc::new(agora_hub::enroll::Lockout::new()),
    );
    let now = agora_hub::server::now_ms();
    let token = signin
        .redeem(&signin.create_code(now).unwrap(), now)
        .unwrap();

    if agora_platform::PTY_SUPPORTED {
        let mut pty = agora_platform::spawn(&agora_platform::PtyConfig {
            argv: vec![
                AGORA.into(),
                "password".into(),
                "set".into(),
                "--hub-dir".into(),
                root.display().to_string(),
            ],
            cwd: None,
            env: vec![
                ("AGORA_HOME".into(), root.display().to_string()),
                ("AGORA_LANG".into(), "en".into()),
            ],
            cols: 120,
            rows: 30,
            limits: agora_platform::ProcessLimits::default(),
            sandbox: None,
            identity: None,
        })
        .unwrap();
        let mut seen = String::new();
        let mut wait_for = |pty: &mut Box<dyn agora_platform::Pty>, text: &str| {
            let deadline = Instant::now() + Duration::from_secs(60);
            while !seen.contains(text) {
                assert!(Instant::now() < deadline, "waited for {text:?}: {seen}");
                if let Ok(b) = pty.output().recv_timeout(Duration::from_millis(100)) {
                    seen.push_str(&String::from_utf8_lossy(&b));
                }
            }
        };
        wait_for(&mut pty, "New screen password");
        pty.write_input(b"open sesame 42\r").unwrap();
        wait_for(&mut pty, "Type it again");
        pty.write_input(b"open sesame 42\r").unwrap();
        wait_for(&mut pty, "open sessions were signed out");
        assert!(pty.wait().unwrap().success(), "{seen}");
        // Ctrl-C while typing stops with 130 and changes nothing (the terminal is restored before it exits).
        let mut stop = agora_platform::spawn(&agora_platform::PtyConfig {
            argv: vec![
                AGORA.into(),
                "password".into(),
                "set".into(),
                "--hub-dir".into(),
                root.display().to_string(),
            ],
            cwd: None,
            env: vec![("AGORA_LANG".into(), "en".into())],
            cols: 120,
            rows: 30,
            limits: agora_platform::ProcessLimits::default(),
            sandbox: None,
            identity: None,
        })
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut text = String::new();
        while !text.contains("New screen password") {
            assert!(Instant::now() < deadline, "{text}");
            if let Ok(b) = stop.output().recv_timeout(Duration::from_millis(100)) {
                text.push_str(&String::from_utf8_lossy(&b));
            }
        }
        stop.write_input(b"half\x03").unwrap();
        assert_eq!(stop.wait().unwrap().code, Some(130), "{text}");
        assert!(
            !seen.contains("open sesame"),
            "the password is never shown: {seen}"
        );
        let password = ScreenPassword::new(&store);
        assert!(password.verify("open sesame 42"));
        assert!(
            !signin.check(&token, agora_hub::server::now_ms()),
            "every session ends"
        );
        assert!(run(&["password", "status"])
            .1
            .contains("A screen password is set"));
    }

    let (code, out) = run(&["password", "clear"]);
    assert_eq!(code, Some(0), "{out}");
    assert!(!ScreenPassword::new(&store).is_set());
    let rows = agora_hub::audit::Audit::new(&store)
        .list(0, None, 50)
        .unwrap();
    assert!(rows.iter().any(|r| r.action == "password_clear"));
}
