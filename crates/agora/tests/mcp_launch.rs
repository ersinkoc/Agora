//! ADR-0133: `agora mcp-launch` against a node's agent socket (a stand-in that answers as the node does): it asks for the server's
//! program, starts it, says its process id, closes it when the node says so, and says how it ended; asked while the node still holds
//! the previous server, it asks again until there is room. Real `agora` processes and a real local socket; the server is the
//! fake agent (never a real program with a real secret).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

mod short_socket;

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

/// `agora-fake-agent` next to the `agora` under test, built once per test binary.
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
        assert!(build.status().expect("run cargo").success());
    });
    p.to_string_lossy().into_owned()
}

/// Reads one JSON line from the launcher's connection.
fn read(r: &mut BufReader<agora_platform::LocalStream>) -> Value {
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    serde_json::from_str(line.trim()).unwrap_or(Value::Null)
}

/// Waits until process `pid` is gone (at most 10 s).
fn gone(pid: u32) {
    let until = Instant::now() + Duration::from_secs(10);
    while agora_platform::process_exists(pid) {
        assert!(Instant::now() < until, "process {pid} still runs");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn launcher(socket: &Path) -> std::process::Child {
    Command::new(AGORA)
        .args(["mcp-launch", "--socket"])
        .arg(socket)
        .args(["db", "L1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

#[test]
fn the_launcher_reports_its_server_closes_it_when_told_and_waits_for_room() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let dir = std::env::temp_dir().join(format!("agora-mcp-launch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sockets = short_socket::SocketDir::new(&dir, "mcpl");
    let socket = sockets.socket("agent-1.sock");
    let listener = agora_platform::local_listen(&socket).unwrap();
    let server = fake_agent();

    // First ask: the node is still letting the previous server go; the launcher asks again on a new connection.
    let mut child = launcher(&socket);
    let first = listener.accept().unwrap();
    let mut r = BufReader::new(first.try_clone().unwrap());
    let ask = read(&mut r);
    assert_eq!(ask["method"], "agora/shared_launch");
    assert_eq!(ask["params"]["launch_id"], "L1");
    let mut w = first;
    writeln!(w, "{}", json!({"jsonrpc":"2.0","id":1,"error":{"code":-32001,"data":{"code":"share.refused","reason":"instance_live"}}})).unwrap();
    drop(w);
    drop(r);

    // Second ask: granted; the server starts and its process id is said on the same connection.
    let second = listener.accept().unwrap();
    let mut r = BufReader::new(second.try_clone().unwrap());
    assert_eq!(read(&mut r)["method"], "agora/shared_launch");
    let mut w = second;
    let probe = dir.join("probe-{pid}.json").to_string_lossy().into_owned();
    writeln!(w, "{}", json!({"jsonrpc":"2.0","id":1,"result":{"command": server, "args": ["--probe-directory", probe], "env": {"MODE": "x"}}})).unwrap();
    let started = read(&mut r);
    assert_eq!(started["method"], "agora/shared_launch_started");
    assert_eq!(started["params"]["item"], "db");
    let pid = started["params"]["pid"].as_u64().unwrap();
    assert!(pid > 0);
    let report = dir.join(format!("probe-{pid}.json"));
    let until = Instant::now() + Duration::from_secs(10);
    while !report.exists() {
        assert!(
            Instant::now() < until,
            "the server the launcher said started is that process"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(child.try_wait().unwrap().is_none(), "the server runs");

    // The node says close: the server is ended and the end is said, then the launcher exits.
    writeln!(
        w,
        "{}",
        json!({"jsonrpc":"2.0","method":"agora/shared_launch_close","params":{"item":"db"}})
    )
    .unwrap();
    let ended = read(&mut r);
    assert_eq!(ended["method"], "agora/shared_launch_ended");
    assert_ne!(
        ended["params"]["code"], 0,
        "ended by the node, not by itself"
    );
    let until = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < until, "the launcher ends with its server");
        std::thread::sleep(Duration::from_millis(50));
    }
    gone(u32::try_from(pid).unwrap());

    // A server that runs the real one as its own child (as `npx` does), through `sh` where there is one: closing it ends the child
    // too, and so does the node's side of the connection ending (the agent ended or the node stopped).
    if Command::new("sh")
        .args(["-c", "true"])
        .status()
        .is_ok_and(|s| s.success())
    {
        let mut child = launcher(&socket);
        let conn = listener.accept().unwrap();
        let mut r = BufReader::new(conn.try_clone().unwrap());
        assert_eq!(read(&mut r)["method"], "agora/shared_launch");
        let mut w = conn;
        let probe = dir.join("deep-{pid}.json").to_string_lossy().into_owned();
        let script = format!("'{server}' --probe-directory '{probe}' & wait");
        writeln!(w, "{}", json!({"jsonrpc":"2.0","id":1,"result":{"command": "sh", "args": ["-c", script], "env": {}}})).unwrap();
        assert_eq!(read(&mut r)["method"], "agora/shared_launch_started");
        let until = Instant::now() + Duration::from_secs(10);
        let deep = loop {
            let found = std::fs::read_dir(&dir).unwrap().flatten().find_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.strip_prefix("deep-")
                    .and_then(|r| r.strip_suffix(".json"))
                    .and_then(|p| p.parse::<u32>().ok())
            });
            if let Some(p) = found {
                break p;
            }
            assert!(Instant::now() < until, "the grandchild started");
            std::thread::sleep(Duration::from_millis(50));
        };
        // The node goes away without a word.
        drop(w);
        drop(r);
        let until = Instant::now() + Duration::from_secs(10);
        while child.try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < until,
                "the launcher ends when the node's side ends"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        gone(deep);
    }
    drop(sockets);
    let _ = std::fs::remove_dir_all(&dir);
}
