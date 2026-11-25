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

/// Audit F1001: `hub.db` is created with the default mode and holds the hashes of the live one-time codes (an 8-letter code is
/// searched for offline within its 10 minutes), so the hub's folder itself is closed to other users: made so by `hub login`,
/// `hub code` and `hub run`, also when an older version left it open.
#[cfg(unix)]
#[test]
fn the_hubs_folder_is_closed_to_other_users() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let root = tmp("private-hub");
    for (name, command) in [("login", "login"), ("code", "code"), ("old", "login")] {
        let hub_dir = root.join(name).join("hub");
        if name == "old" {
            std::fs::create_dir_all(&hub_dir).unwrap();
            std::fs::set_permissions(&hub_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let out = agora(&root, &["hub", command])
            .arg("--hub-dir")
            .arg(&hub_dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "{name}: {out:?}");
        assert!(hub_dir.join("hub.db").exists(), "{name}");
        assert_eq!(mode(&hub_dir), 0o700, "{name}: the hub's folder is open");
    }
    let hub_dir = root.join("run").join("hub");
    let (hub, _addr) = start_hub(&root.join("run"), &hub_dir, "127.0.0.1:0");
    let _hub = Proc(hub);
    assert_eq!(mode(&hub_dir), 0o700, "hub run: the hub's folder is open");
    let _ = std::fs::remove_dir_all(root);
}

/// ADR-0112: `agora hub fleet` says whether agents see every machine and agent; the switch matters once the hub has people, and a
/// machine without a hub folder is told so (nothing is created).
#[test]
fn the_fleet_view_is_turned_on_and_off_from_the_command_line() {
    let root = tmp("fleet");
    let run = |dir: &Path, args: &[&str]| {
        let o = agora(&root, args)
            .arg("--hub-dir")
            .arg(dir)
            .output()
            .unwrap();
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    let warning = "is on for this hub";
    let (code, out, _) = run(&root, &["hub", "fleet", "status"]);
    assert!(
        code == Some(0) && out.contains("nobody but you") && !out.contains("switch is on"),
        "{out}"
    );
    // F2 of the #1205 review: on a solo hub the stored switch is said too, and adding a person while it is on warns.
    let (code, out, _) = run(&root, &["hub", "fleet", "on"]);
    assert!(
        code == Some(0) && out.contains("nobody but you") && out.contains("The switch is on"),
        "{out}"
    );
    let (code, _, err) = run(&root, &["hub", "user", "add", "Eda", "--role", "member"]);
    assert!(code == Some(0) && err.contains(warning), "{err}");
    let (code, out, _) = run(&root, &["hub", "fleet", "status"]);
    assert!(code == Some(0) && out.contains("also in projects"), "{out}");
    let (code, out, _) = run(&root, &["hub", "fleet", "off"]);
    assert!(
        code == Some(0) && out.contains("only the machines and agents of their own projects"),
        "{out}"
    );
    let (code, _, err) = run(&root, &["hub", "user", "add", "Bo", "--role", "viewer"]);
    assert!(code == Some(0) && !err.contains(warning), "{err}");
    let (code, out, _) = run(&root, &["hub", "fleet", "status"]);
    assert!(code == Some(0) && out.contains("own projects"), "{out}");
    for bad in [
        &["hub", "fleet", "maybe"][..],
        &["hub", "fleet"],
        &["hub", "fleet", "on", "off"],
    ] {
        let (code, _, err) = run(&root, bad);
        assert!(
            code == Some(2) && err.contains("arguments"),
            "{bad:?}: {err}"
        );
    }
    let none = root.join("no-hub");
    let (code, _, _) = run(&none, &["hub", "fleet", "status"]);
    assert_eq!(code, Some(1));
    assert!(!none.exists(), "a hub folder was made");
    let _ = std::fs::remove_dir_all(&root);
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
    // A person sees and works in the projects they were granted, `MACHINE/PROJECT` as the window names them, and nothing else.
    seed_projects(&root);
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
            "user.grant:ayse n1/#web",
            "user.revoke:ayse n1/#web",
            "user.remove:ayse"
        ],
        "{out}"
    );
    let (ok, out, _) = run(&["hub", "audit", "export", "--who", "nobody"]);
    assert!(ok && out.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

/// What the hub has heard from two machines (#904): `n1` is "laptop" with a git project `web` (id `#web`), a folder `notes` that is no
/// repository (`id:notes`) and two different projects both called `api`; `n2` and `n3` are both called "box" and each has an `ops`.
fn seed_projects(hub_dir: &Path) {
    use agora_hub::store::EventStore;
    use agora_protocol::v1::{
        event::Body, CheckoutDiscovered, Event, NodeOnline, WorkspaceRegistered,
    };
    use prost::Message;
    let store = SqliteStore::open(&hub_dir.join("hub.db")).unwrap();
    let mut seqs = std::collections::BTreeMap::new();
    let mut put = |node: &str, body: Body| {
        let seq = seqs.entry(node.to_string()).or_insert(0u64);
        *seq += 1;
        let ev = Event {
            producer_id: node.into(),
            seq: *seq,
            at_ms: 1,
            body: Some(body),
        };
        store.append(node, *seq, 1, &ev.encode_to_vec()).unwrap();
    };
    let online = |node: &str, host: &str| {
        Body::NodeOnline(NodeOnline {
            node_id: node.into(),
            hostname: host.into(),
            ..Default::default()
        })
    };
    let checkout = |node: &str, id: &str, project: &str, path: &str| {
        Body::CheckoutDiscovered(CheckoutDiscovered {
            checkout_id: id.into(),
            node_id: node.into(),
            project_id: project.into(),
            path: path.into(),
        })
    };
    put("n1", online("n1", "laptop"));
    put("n1", checkout("n1", "c1", "#web", "/home/a/web"));
    put("n1", checkout("n1", "c2", "#api1", "/home/a/api"));
    put("n1", checkout("n1", "c3", "#api2", "/srv/api/"));
    put(
        "n1",
        Body::WorkspaceRegistered(WorkspaceRegistered {
            workspace_id: "w1".into(),
            node_id: "n1".into(),
            project_id: "id:notes".into(),
            path: "/home/a/notes".into(),
            ..Default::default()
        }),
    );
    put("n2", online("n2", "box"));
    put("n2", checkout("n2", "c1", "#ops2", "/opt/ops"));
    put("n3", online("n3", "box"));
    put("n3", checkout("n3", "c1", "#ops3", "/opt/ops"));
}

#[test]
fn a_project_is_granted_by_the_name_the_window_shows_and_an_unknown_or_ambiguous_one_is_refused() {
    let root = tmp("grant-names");
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
    let (ok, _, err) = run(&["hub", "user", "add", "Ayse"]);
    assert!(ok, "{err}");
    seed_projects(&root);
    let grants = || {
        let store = SqliteStore::open(&root.join("hub.db")).unwrap();
        agora_hub::users::Users::new(&store)
            .grants_of("ayse")
            .unwrap()
    };
    // The owner can list what may be granted, by the names the window shows (never an internal id).
    let (ok, out, err) = run(&["hub", "user", "projects"]);
    assert!(ok, "{err}");
    // Two machines share the name "box": each is listed by what tells them apart.
    for line in [
        "laptop/web",
        "laptop/notes",
        "laptop/api",
        "/home/a/api",
        "/srv/api",
        "n2/ops",
        "n3/ops",
        "box",
    ] {
        assert!(out.contains(line), "{line}: {out}");
    }
    assert!(!out.contains("#web") && !out.contains("id:notes"), "{out}");
    // By the machine and folder name the window shows: the hub stores the project's id.
    let (ok, _, err) = run(&["hub", "user", "grant", "ayse", "laptop/web"]);
    assert!(ok, "{err}");
    let (ok, _, err) = run(&["hub", "user", "grant", "ayse", "laptop/notes"]);
    assert!(ok, "{err}");
    assert_eq!(grants(), ["n1/#web", "n1/id:notes"]);
    // A name the hub has never seen is refused with a reason, and nothing is stored.
    let (ok, _, err) = run(&["hub", "user", "grant", "ayse", "laptop/proj"]);
    assert!(
        !ok && err.contains("proj") && err.contains("agora hub user projects"),
        "{err}"
    );
    let (ok, _, err) = run(&["hub", "user", "grant", "ayse", "desk/web"]);
    assert!(!ok && err.contains("desk"), "{err}");
    assert_eq!(grants(), ["n1/#web", "n1/id:notes"]);
    // Two projects called `api`: refused, and the person is shown how to choose (by the folder).
    let (ok, _, err) = run(&["hub", "user", "grant", "ayse", "laptop/api"]);
    assert!(
        !ok && err.contains("/home/a/api") && err.contains("/srv/api"),
        "{err}"
    );
    assert_eq!(grants().len(), 2);
    let (ok, _, err) = run(&["hub", "user", "grant", "ayse", "laptop", "/srv/api"]);
    assert!(ok, "{err}");
    assert!(grants().contains(&"n1/#api2".to_string()));
    // Two machines called `box`: refused, and the machine is named by its id.
    let (ok, _, err) = run(&["hub", "user", "grant", "ayse", "box/ops"]);
    assert!(!ok && err.contains("n2") && err.contains("n3"), "{err}");
    let (ok, _, err) = run(&["hub", "user", "grant", "ayse", "n3/ops"]);
    assert!(ok, "{err}");
    assert!(grants().contains(&"n3/#ops3".to_string()));
    // An id the hub knows still works.
    let (ok, _, err) = run(&["hub", "user", "grant", "ayse", "n1/#api1"]);
    assert!(ok, "{err}");
    // The list says where a person works by name.
    let (ok, out, _) = run(&["hub", "user", "list"]);
    assert!(
        ok && out.contains("works in laptop/web") && out.contains("works in laptop/notes"),
        "{out}"
    );
    assert!(!out.contains("#web"), "{out}");
    // Two of the projects are called `api` there: each is told apart by its folder.
    assert!(
        out.contains("laptop/api (/home/a/api)") && out.contains("laptop/api (/srv/api/)"),
        "{out}"
    );
    // A revoke by name, and of a row an older build stored without checking (it shows nothing, but must be removable).
    let (ok, _, err) = run(&["hub", "user", "revoke", "ayse", "laptop/notes"]);
    assert!(ok, "{err}");
    {
        let store = SqliteStore::open(&root.join("hub.db")).unwrap();
        assert!(agora_hub::users::Users::new(&store)
            .grant("ayse", "laptop", "proj")
            .unwrap());
    }
    let (ok, out, _) = run(&["hub", "user", "list"]);
    assert!(ok && out.contains("works in laptop/proj"), "{out}");
    let (ok, _, err) = run(&["hub", "user", "revoke", "ayse", "laptop/proj"]);
    assert!(ok, "{err}");
    assert_eq!(grants(), ["n1/#api1", "n1/#api2", "n1/#web", "n3/#ops3"]);
    // Revoking what the person does not have says so (not that the person is unknown).
    let (ok, _, err) = run(&["hub", "user", "revoke", "ayse", "laptop/notes"]);
    assert!(!ok && !err.contains("no team member"), "{err}");
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
fn the_hubs_first_snapshot_reaches_a_client_that_sends_a_command_at_once() {
    // Seen on main after #446: `agora tui` registers its device right after connecting, and the hub's first snapshot arrived while
    // that command waited for its answer; the wait dropped it, the hub never sends it again, and the console stayed at
    // "Connecting…". A client that asks something at once must still get the snapshot.
    use agora_tui::client::{sign_in, Hub, Session, Update};
    let root = tmp("first-snapshot");
    let (hub_dir, home) = (root.join("hub"), root.join("home"));
    std::fs::create_dir_all(&home).unwrap();
    let mut hub = agora(
        &home,
        &["hub", "run", "--listen", "127.0.0.1:0", "--hub-dir"],
    )
    .arg(&hub_dir)
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    let mut line = String::new();
    BufReader::new(hub.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let _hub = Proc(hub);
    let addr = line
        .split("running on ")
        .nth(1)
        .and_then(|r| r.split(". Its").next())
        .expect(&line)
        .trim()
        .to_string();
    let login = agora(&home, &["hub", "login", "--hub-dir"])
        .arg(&hub_dir)
        .output()
        .unwrap();
    let target = Hub {
        addr,
        ca_pem: std::fs::read_to_string(hub_dir.join("ca.pem")).unwrap(),
    };
    let token = sign_in(&target, &code_in(&String::from_utf8_lossy(&login.stdout))).unwrap();
    let mut session = Session::connect(&target, &token).unwrap();
    // A command right away, as the console's device registration is: its answer comes after the snapshot.
    let _ = session.list_devices();
    let end = std::time::Instant::now() + Duration::from_secs(10);
    let mut got = false;
    while !got && std::time::Instant::now() < end {
        got = matches!(
            session.next(Duration::from_millis(200)),
            Update::Snapshot(_)
        );
    }
    assert!(got, "the first snapshot was lost while the command waited");
    let _ = std::fs::remove_dir_all(root);
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
fn the_screen_password_is_never_read_from_a_pipe_and_clearing_it_signs_out() {
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

    // Runs on every system (Windows included, where the echo-less reader is only compiled): a pipe is refused, and clearing a
    // password ends every session.
    ScreenPassword::with_iterations(&SqliteStore::open(&root.join("hub.db")).unwrap(), 1_000)
        .set("open sesame 42")
        .unwrap();
    // A live session, to see it end when the password is cleared.
    let store = SqliteStore::open(&root.join("hub.db")).unwrap();
    let signin = agora_hub::signin::SignIn::new(
        &store,
        std::sync::Arc::new(agora_hub::enroll::Lockout::new()),
    );
    let now = agora_hub::server::now_ms();
    let token = signin
        .redeem(&signin.create_code(now).unwrap(), now)
        .unwrap();

    let (code, out) = run(&["password", "clear"]);
    assert_eq!(code, Some(0), "{out}");
    assert!(!ScreenPassword::new(&store).is_set());
    assert!(
        !signin.check(&token, agora_hub::server::now_ms()),
        "every session ends"
    );
    let rows = agora_hub::audit::Audit::new(&store)
        .list(0, None, 50)
        .unwrap();
    assert!(rows.iter().any(|r| r.action == "password_clear"));
}

/// Typing the password in a real terminal: nothing is echoed, Ctrl-C stops with 130. It drives a Unix pty; on Windows the reader is
/// compiled and the pipe refusal above is what runs.
#[cfg(unix)]
#[test]
fn the_screen_password_is_typed_twice_unseen_and_ctrl_c_stops_it() {
    use agora_hub::password::ScreenPassword;
    let root = tmp("password-pty");
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
    let store = SqliteStore::open(&root.join("hub.db")).unwrap();
    let signin = agora_hub::signin::SignIn::new(
        &store,
        std::sync::Arc::new(agora_hub::enroll::Lockout::new()),
    );
    let now = agora_hub::server::now_ms();
    let token = signin
        .redeem(&signin.create_code(now).unwrap(), now)
        .unwrap();
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
        env_remove: Vec::new(),
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
    wait_for(&mut pty, "signed out.");
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
        env_remove: Vec::new(),
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

/// Appends to `file` (made if missing, with its folder): where a test keeps a process's output to print when it fails.
fn log_to(file: &Path) -> std::fs::File {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .unwrap()
}

/// Prints the files (the hub's and the node's output) when the test fails: why a node did not connect is in them (#957).
struct ShowOnFailure(Vec<PathBuf>);

impl Drop for ShowOnFailure {
    fn drop(&mut self) {
        if std::thread::panicking() {
            for f in &self.0 {
                let text = std::fs::read_to_string(f).unwrap_or_default();
                eprintln!("--- {}:\n{text}", f.display());
            }
        }
    }
}

/// Starts `agora hub run` on `listen` over `hub_dir`; returns the process and the address it says it runs on. Everything the hub
/// prints is appended to `<hub_dir>.log`.
fn start_hub(home: &Path, hub_dir: &Path, listen: &str) -> (Child, String) {
    use std::io::Write;
    let log = hub_dir.with_extension("log");
    let mut hub = agora(home, &["hub", "run", "--listen", listen, "--hub-dir"])
        .arg(hub_dir)
        .stdout(Stdio::piped())
        .stderr(log_to(&log))
        .spawn()
        .unwrap();
    let mut line = String::new();
    let mut out = BufReader::new(hub.stdout.take().unwrap());
    out.read_line(&mut line).unwrap();
    let mut keep = log_to(&log);
    let _ = keep.write_all(line.as_bytes());
    // Keep reading what the hub prints so it never blocks on a full pipe.
    std::thread::spawn(move || std::io::copy(&mut out, &mut keep));
    let addr = line
        .split("running on ")
        .nth(1)
        .and_then(|r| r.split(". Its").next())
        .expect(&line)
        .trim()
        .to_string();
    (hub, addr)
}

/// What a signed-in window is shown first: the hub's snapshot.
fn first_snapshot(home: &Path, hub_dir: &Path, addr: &str) -> agora_protocol::v1::Snapshot {
    use agora_tui::client::{sign_in, Hub, Session, Update};
    let login = agora(home, &["hub", "login", "--hub-dir"])
        .arg(hub_dir)
        .output()
        .unwrap();
    let target = Hub {
        addr: addr.to_string(),
        ca_pem: std::fs::read_to_string(hub_dir.join("ca.pem")).unwrap(),
    };
    let token = sign_in(&target, &code_in(&String::from_utf8_lossy(&login.stdout))).unwrap();
    let mut session = Session::connect(&target, &token).unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if let Update::Snapshot(s) = session.next(Duration::from_millis(200)) {
            return *s;
        }
    }
    panic!("no snapshot from the hub at {addr}");
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[test]
fn a_machine_that_has_not_come_back_after_the_hub_is_killed_is_shown_offline() {
    // Seen on the maintainer's hub (ADR-0070, #467): the view is rebuilt from the log, so a machine whose connection was open
    // when the hub was killed (no end of it was ever recorded) stayed online after the hub started again, until it redialled,
    // or forever when it never did. Presence is about live connections, and a hub that has just started has none.
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let root = tmp("presence-at-open");
    let (hub_dir, home, node_home) = (root.join("hub"), root.join("home"), root.join("node"));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&node_home).unwrap();
    // Dropped last, after the processes: on a failure the hub's and the node's whole output is printed.
    let _show = ShowOnFailure(vec![root.join("hub.log"), root.join("node.log")]);

    let (hub, addr) = start_hub(&home, &hub_dir, "127.0.0.1:0");
    let mut hub = Proc(hub);
    let port = addr.rsplit(':').next().unwrap().to_string();
    let out = agora(&home, &["hub", "code", "--hub-dir"])
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
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let node = agora(&node_home, &["node", "run"])
        .stdout(log_to(&root.join("node.log")))
        .stderr(log_to(&root.join("node.log")))
        .spawn()
        .unwrap();
    let _node = Proc(node);
    let online = |s: &agora_protocol::v1::Snapshot| {
        s.nodes
            .iter()
            .find(|n| n.node_id == "laptop")
            .map(|n| (n.online, n.last_event_ms))
    };
    let end = Instant::now() + Duration::from_secs(30);
    while online(&first_snapshot(&home, &hub_dir, &addr)).map(|o| o.0) != Some(true) {
        assert!(Instant::now() < end, "the node never came online");
        std::thread::sleep(Duration::from_millis(200));
    }

    // The hub is killed (SIGKILL on Unix, this exact process): it records nothing about the node's connection.
    hub.0.kill().unwrap();
    hub.0.wait().unwrap();
    let killed_at = now_ms();
    // It starts again on the same directory, on another port, so the node cannot redial it: the machine is shown offline, with
    // "last seen" from before the kill, not the restart.
    let (again, other) = start_hub(&home, &hub_dir, "127.0.0.1:0");
    let mut again = Proc(again);
    assert_ne!(other, addr);
    // #508: the restarted hub rewrites its `listen` file; only `agora tui --hub-dir` reads it, never the node (it dials the
    // address it enrolled with), so the node stays unable to reach this hub.
    let listen = || {
        std::fs::read_to_string(hub_dir.join("listen"))
            .unwrap()
            .trim()
            .to_string()
    };
    assert_eq!(listen(), other);
    let (up, seen) = online(&first_snapshot(&home, &hub_dir, &other)).unwrap();
    assert!(!up, "a machine that has not come back is shown offline");
    assert!(
        seen > 0 && seen <= killed_at,
        "last seen {seen} vs killed at {killed_at}"
    );

    // Killed again and started where the node dials: it comes back online by itself.
    again.0.kill().unwrap();
    again.0.wait().unwrap();
    let (back, addr) = start_hub(&home, &hub_dir, &format!("127.0.0.1:{port}"));
    let _back = Proc(back);
    assert_eq!(listen(), addr);
    let end = Instant::now() + Duration::from_secs(30);
    while online(&first_snapshot(&home, &hub_dir, &addr)).map(|o| o.0) != Some(true) {
        assert!(Instant::now() < end, "the node never came back online");
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = std::fs::remove_dir_all(root);
}

/// `agora tui --hub-dir DIR` on the hub's machine, without `--hub` and without `machine.toml`: it dials the port the hub in that
/// folder really listens on, not the default 7443 (found on the maintainer's hub, which runs on a port of its own).
#[test]
fn the_console_with_only_the_hub_folder_dials_the_hubs_real_port() {
    let root = tmp("tui-port");
    let (hub_dir, home) = (root.join("hub"), root.join("home"));
    std::fs::create_dir_all(&home).unwrap();
    let mut hub = agora(
        &home,
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
    assert_ne!(
        addr, "127.0.0.1:7443",
        "the test needs a port other than the default"
    );

    let mut tui = agora(&home, &["tui", "--hub-dir"])
        .arg(&hub_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut err = BufReader::new(tui.stderr.take().unwrap());
    let _tui = Proc(tui);
    let mut first = String::new();
    err.read_line(&mut first).unwrap();
    assert!(first.contains(&format!("Connecting to {addr}")), "{first}");
}

/// ADR-0073 from the command line: a machine the hub has not seen for days is listed as old; `forget --old --yes` has the running
/// hub forget it, and it is listed no more.
#[test]
fn an_old_machine_is_listed_as_old_and_forgotten_through_the_running_hub() {
    use agora_hub::store::EventStore;
    use agora_protocol::v1::{event::Body, Event, NodeOnline};
    use prost::Message;
    let root = tmp("machines");
    let (hub_dir, home) = (root.join("hub"), root.join("home"));
    std::fs::create_dir_all(&hub_dir).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    {
        // A machine last heard from in 2020.
        let store = SqliteStore::open(&hub_dir.join("hub.db")).unwrap();
        let ev = Event {
            producer_id: "old-box".into(),
            seq: 1,
            at_ms: 1_600_000_000_000,
            body: Some(Body::NodeOnline(NodeOnline {
                node_id: "old-box".into(),
                ..Default::default()
            })),
        };
        store
            .append("old-box", 1, ev.at_ms, &ev.encode_to_vec())
            .unwrap();
    }
    let mut hub = agora(
        &home,
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
    let run = |args: &[&str]| {
        let o = agora(&home, args)
            .arg("--hub-dir")
            .arg(&hub_dir)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (
            o.status.success(),
            String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr),
        )
    };
    let (ok, listed) = run(&["hub", "machines"]);
    assert!(ok, "{listed}");
    assert!(
        listed.contains("old-box · old, last seen 2020-"),
        "{listed}"
    );
    // Without a terminal to answer and without --yes, nothing is forgotten.
    let (_, asked) = run(&["hub", "machines", "forget", "--old"]);
    assert!(asked.contains("Nothing was forgotten"), "{asked}");
    let (ok, forgot) = run(&["hub", "machines", "forget", "--old", "--yes"]);
    assert!(ok, "{forgot}");
    assert!(forgot.contains("old-box is forgotten"), "{forgot}");
    let (_, after) = run(&["hub", "machines"]);
    assert!(after.contains("The hub shows no machine"), "{after}");
}

/// On a machine that only runs a node there is no hub folder: `agora hub machines` and the other commands that read a running hub's
/// files say so, instead of "unable to open database file". The ones that create the hub folder themselves are untouched.
#[test]
fn a_hub_command_on_a_machine_without_a_hub_says_so() {
    let home = tmp("not-here");
    for cmd in ["fingerprint", "audit", "devices", "machines"] {
        let out = agora(&home, &["hub", cmd]).output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{cmd}: {err}");
        assert!(
            err.contains("This machine does not run the hub"),
            "{cmd}: {err}"
        );
        assert!(!err.contains("database"), "{cmd}: {err}");
        assert!(!home.join("hub").exists(), "{cmd} made a hub folder");
    }
    // `hub login` is not read-only (it makes the folder and a sign-in code), so it is not guarded and works as before.
    let fresh = tmp("not-here-login");
    let out = agora(&fresh, &["hub", "login"]).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(fresh.join("hub").is_dir());
    let _ = std::fs::remove_dir_all(&fresh);
    // `hub code` still makes the folder and a code before the hub's first start, as before.
    let out = agora(&home, &["hub", "code"]).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(home.join("hub").is_dir());
    // With a hub folder, the commands run as before (an empty hub has no machines to list).
    let out = agora(&home, &["hub", "machines"]).output().unwrap();
    assert!(!String::from_utf8_lossy(&out.stderr).contains("does not run the hub"));
    let _ = std::fs::remove_dir_all(&home);
}

/// ADR-0076: a metrics token is made, listed without being shown, and revoked from the command line.
#[test]
fn a_metrics_token_is_made_listed_and_revoked_from_the_command_line() {
    let root = tmp("metrics-token");
    let hub = root.join("hub");
    let run = |args: &[&str]| {
        let mut full = vec!["hub", "metrics-token"];
        full.extend_from_slice(args);
        full.extend(["--hub-dir", hub.to_str().unwrap()]);
        agora(&root, &full).output().unwrap()
    };
    // A wrong command line makes nothing, not even the hub folder.
    assert_eq!(run(&["frob"]).status.code(), Some(2));
    assert_eq!(run(&["list", "--expires", "3"]).status.code(), Some(2));
    assert_eq!(
        run(&["create", "prom", "--expires", "0"]).status.code(),
        Some(2)
    );
    assert!(!hub.exists());
    let out = run(&["create", "prom", "--expires", "30"]);
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let token = text
        .split_whitespace()
        .find(|w| w.starts_with("agm_"))
        .unwrap()
        .trim_end_matches(':')
        .to_string();
    assert_eq!(token.len(), 4 + 64, "{text}");
    assert_eq!(run(&["create", "prom"]).status.code(), Some(1));
    let listed = String::from_utf8(run(&["list"]).stdout).unwrap();
    assert!(
        listed.contains("prom") && !listed.contains(&token[4..]),
        "{listed}"
    );
    assert!(!listed.contains("expires never"), "{listed}");
    assert!(run(&["revoke", "prom"]).status.success());
    assert_eq!(run(&["revoke", "prom"]).status.code(), Some(1));
    assert!(String::from_utf8(run(&["list"]).stdout)
        .unwrap()
        .contains("revoked"));
    let _ = std::fs::remove_dir_all(&root);
}
