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
