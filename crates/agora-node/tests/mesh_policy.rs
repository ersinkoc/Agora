//! The owner's `mesh.remote` policy (ADR-0031): off means nothing crosses between nodes, in either direction, while
//! messages on this node keep working. A typo in the policy file closes the door rather than opening it.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};

use agora_core::AgentId;
use agora_mesh::broker::{AgentInfo, Limits};
use agora_node::daemon::{read_policy, Policy};
use agora_node::mesh_server::MeshServer;

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("agora-policy-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn the_policy_file_defaults_to_allowed_and_a_bad_value_means_denied() {
    let dir = tmp("file");
    let file = dir.join("policy");
    assert_eq!(read_policy(&file), Policy::default(), "no file");
    for (text, allowed) in [
        ("mesh.remote = true\n", true),
        ("# off\nmesh.remote = false\n", false),
        ("mesh.remote=false", false),
        ("mesh.remote = flase\n", false),
        ("mesh.remote =\n", false),
        ("other = 1\n", true),
    ] {
        std::fs::write(&file, text).unwrap();
        assert_eq!(read_policy(&file).mesh_remote, allowed, "{text:?}");
    }
}

#[test]
fn the_sandbox_lines_are_read_and_a_typo_is_the_strictest_level() {
    use agora_node::sandbox::Level;
    let dir = tmp("sandbox");
    let file = dir.join("policy");
    // Absolute on every system: the temp folder's own path.
    let (a, b, c) = (dir.join("a"), dir.join("b"), dir.join("c"));
    std::fs::write(
        &file,
        format!(
            "sandbox = worktree\nsandbox.read = {}, {}\nsandbox.write = {}\n",
            a.display(),
            b.display(),
            c.display()
        ),
    )
    .unwrap();
    let p = read_policy(&file).sandbox;
    assert_eq!(p.level, Level::Worktree);
    assert_eq!(p.read, vec![a, b]);
    assert_eq!(p.write, vec![c]);
    assert!(p.trusted_kinds.is_empty());
    std::fs::write(&file, "sandbox.trust_manifest = claude-code, codex\n").unwrap();
    assert_eq!(
        read_policy(&file).sandbox.trusted_kinds,
        ["claude-code", "codex"]
    );
    std::fs::write(&file, "sandbox = wortree\n").unwrap();
    assert_eq!(read_policy(&file).sandbox.level, Level::WorktreeOffline);
    assert_eq!(read_policy(&dir.join("none")).sandbox.level, Level::Off);
}

#[test]
fn sandbox_paths_may_start_at_the_home_folder_and_relative_or_parent_paths_are_dropped() {
    let dir = tmp("sandbox-home");
    let file = dir.join("policy");
    std::fs::write(
        &file,
        "sandbox.read = ~/.gitconfig, ~/.rustup, relative/dir, ~/../escape\nsandbox.write = ~/.cargo/registry, ./here\n",
    )
    .unwrap();
    let home = agora_node::usage_files::home_dir().expect("a home folder");
    let p = read_policy(&file).sandbox;
    assert_eq!(p.read, vec![home.join(".gitconfig"), home.join(".rustup")]);
    assert_eq!(p.write, vec![home.join(".cargo/registry")]);
}

#[test]
fn with_remote_denied_outgoing_messages_fail_with_a_policy_notice_and_incoming_ones_are_refused() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let dir = tmp("deny");
    let server = MeshServer::new(&dir.join("m"), Limits::default()).unwrap();
    server.set_remote_allowed(false);
    let a1 = AgentId::new("a1");
    let socket = server
        .register(AgentInfo {
            id: a1.clone(),
            kind: "shell".into(),
            node: "n1".into(),
            project: "p".into(),
            label: String::new(),
            worktree: String::new(),
            state: "idle".into(),
        })
        .unwrap();
    let mut conn = agora_platform::local_connect(&socket).unwrap();
    let mut reader = BufReader::new(conn.try_clone().unwrap());
    let mut call = |line: serde_json::Value| {
        writeln!(conn, "{line}").unwrap();
        let mut out = String::new();
        reader.read_line(&mut out).unwrap();
        out
    };
    call(
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}),
    );
    let sent = call(
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "send_message", "arguments": {"to": "n2/b1", "body": "x"}}}),
    );
    let inbox = server.mesh().lock().unwrap().inbox(&a1, None, 10).unwrap();
    assert_eq!(
        (inbox[0].kind, inbox[0].body.as_str()),
        ("delivery_failed", "node.policy_denied")
    );
    // The notice names the message that failed: the id `send_message` answered with.
    let failed = inbox[0].reply_to.clone().expect("the failed message's id");
    let sent: serde_json::Value = serde_json::from_str(&sent).unwrap();
    let text = sent["result"]["content"][0]["text"].as_str().unwrap();
    let answer: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        answer["message_id"].as_str(),
        Some(failed.as_str()),
        "{sent}"
    );
    let err = server
        .deliver_remote("n2", "b1", &a1, "hi", &[])
        .unwrap_err();
    assert_eq!(err.code.as_str(), "node.policy_denied");
}

#[test]
fn the_signed_commands_line_lists_commands_and_an_absent_or_empty_one_lists_none() {
    let dir = tmp("signed");
    let file = dir.join("policy");
    assert!(!read_policy(&file).signed_commands.requires("agent.prompt"));
    std::fs::write(
        &file,
        "signed_commands = agent.start, terminal.*, ,files.write\nmesh.remote = false\n",
    )
    .unwrap();
    let p = read_policy(&file);
    assert!(!p.mesh_remote, "other lines still read");
    for yes in [
        "agent.start",
        "terminal.input",
        "terminal.open",
        "files.write",
    ] {
        assert!(p.signed_commands.requires(yes), "{yes}");
    }
    for no in ["agent.prompt", "files.read", "git.diff"] {
        assert!(!p.signed_commands.requires(no), "{no}");
    }
    std::fs::write(&file, "signed_commands =\n").unwrap();
    assert!(!read_policy(&file).signed_commands.requires("agent.start"));
}
