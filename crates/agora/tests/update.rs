//! `agora update` against a stand-in release server on loopback: the program is only replaced by a release whose checksum and
//! signature check out, and every refusal leaves it as it was. The copy of `agora` under test is a scratch file, never the real one.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

fn sha256_hex(bytes: &[u8]) -> String {
    // `sha256sum` (Linux) or `shasum -a 256` (macOS), fed on stdin.
    let (tool, args): (&str, &[&str]) =
        if Command::new("sha256sum").arg("--version").output().is_ok() {
            ("sha256sum", &[])
        } else {
            ("shasum", &["-a", "256"])
        };
    let mut child = Command::new(tool)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

/// A server for a fixed set of paths; `/releases/latest` redirects to the tag page of `latest`.
fn serve(files: HashMap<String, Vec<u8>>, latest: &str) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let files = Arc::new(files);
    let latest = latest.to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let mut buf = [0u8; 2048];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
            let reply: Vec<u8> = if path == "/releases/latest" {
                format!("HTTP/1.1 302 Found\r\nLocation: /releases/tag/v{latest}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
            } else if path.starts_with("/releases/tag/") {
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
            } else if let Some(body) = files.get(&path) {
                let mut r = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .into_bytes();
                r.extend_from_slice(body);
                r
            } else {
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
            };
            let _ = s.write_all(&reply);
        }
    });
    port
}

fn triple_and_ext() -> Option<(&'static str, &'static str)> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some(("x86_64-unknown-linux-gnu", "")),
        ("linux", "aarch64") => Some(("aarch64-unknown-linux-gnu", "")),
        ("macos", "x86_64") => Some(("x86_64-apple-darwin", "")),
        ("macos", "aarch64") => Some(("aarch64-apple-darwin", "")),
        _ => None,
    }
}

fn keygen(dir: &Path, name: &str) -> PathBuf {
    let key = dir.join(name);
    assert!(Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "t", "-f"])
        .arg(&key)
        .status()
        .unwrap()
        .success());
    key
}

/// The `allowed_signers` file for a key: principal `agora-release` and its public half.
fn allowed(dir: &Path, key: &Path, file: &str) -> PathBuf {
    let public = std::fs::read_to_string(format!("{}.pub", key.display())).unwrap();
    let parts: Vec<&str> = public.split_whitespace().collect();
    let path = dir.join(file);
    std::fs::write(&path, format!("agora-release {} {}\n", parts[0], parts[1])).unwrap();
    path
}

fn sign(file: &Path, key: &Path) -> Vec<u8> {
    assert!(Command::new("ssh-keygen")
        .args(["-Y", "sign", "-n", "file", "-f"])
        .arg(key)
        .arg(file)
        .output()
        .unwrap()
        .status
        .success());
    let sig = std::fs::read(format!("{}.sig", file.display())).unwrap();
    std::fs::remove_file(format!("{}.sig", file.display())).unwrap();
    sig
}

#[test]
fn a_release_replaces_the_program_only_when_its_checksum_and_signature_check_out() {
    let Some((triple, ext)) = triple_and_ext() else {
        return;
    };
    if Command::new("curl").arg("--version").output().is_err()
        || Command::new("ssh-keygen").arg("-?").output().is_err()
    {
        return;
    }
    let dir = std::env::temp_dir().join(format!("agora-update-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let new_program = b"#!/bin/sh\necho updated-program\n".to_vec();
    let asset = dir.join("asset");
    std::fs::write(&asset, &new_program).unwrap();
    let release_key = keygen(&dir, "release");
    let other_key = keygen(&dir, "other");
    let signature = sign(&asset, &release_key);

    let name = format!("agora-{triple}{ext}");
    let mut files = HashMap::new();
    files.insert(
        format!("/releases/download/v9.9.9/{name}"),
        new_program.clone(),
    );
    files.insert(
        format!("/releases/download/v9.9.9/{name}.sha256"),
        format!("{}  {name}\n", sha256_hex(&new_program)).into_bytes(),
    );
    files.insert(format!("/releases/download/v9.9.9/{name}.sig"), signature);
    // A release whose checksum file is wrong.
    files.insert(
        format!("/releases/download/v9.9.8/{name}"),
        new_program.clone(),
    );
    files.insert(
        format!("/releases/download/v9.9.8/{name}.sha256"),
        format!("{}  {name}\n", "0".repeat(64)).into_bytes(),
    );
    let port = serve(files, "9.9.9");
    let base = format!("http://127.0.0.1:{port}/releases/download");

    // The program under test: a copy, so the real binary is never replaced.
    let program = dir.join("agora-under-test");
    let reset = || {
        std::fs::copy(AGORA, &program).unwrap();
        std::fs::read(&program).unwrap()
    };
    let original = reset();
    let run = |args: &[&str], home: &Path| {
        Command::new(&program)
            .arg("update")
            .args(args)
            .args(["--release-url", &base])
            .env("AGORA_HOME", home)
            .env("LC_ALL", "en_US.UTF-8")
            .output()
            .unwrap()
    };
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();

    // The newest release is found through the `latest` redirect; --check changes nothing.
    let out = run(&["--check"], &home);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && text.contains("9.9.9"),
        "{text} {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read(&program).unwrap(), original);

    // No pinned key and no --unsigned: refused.
    let out = run(&[], &home);
    assert!(
        !out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(std::fs::read(&program).unwrap(), original);

    // A wrong checksum: refused.
    let out = run(&["--version", "9.9.8", "--unsigned"], &home);
    assert!(
        !out.status.success() && String::from_utf8_lossy(&out.stderr).contains("checksum"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read(&program).unwrap(), original);

    // A key that did not sign it: refused.
    let wrong = allowed(&dir, &other_key, "wrong-signers");
    let out = run(&["--release-key", &wrong.to_string_lossy()], &home);
    assert!(
        !out.status.success() && String::from_utf8_lossy(&out.stderr).contains("signature"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read(&program).unwrap(), original);

    // The pinned key signed it: the program is replaced (and still runs).
    let right = allowed(&dir, &release_key, "release-key");
    std::fs::copy(&right, home.join("release-key")).unwrap();
    let out = run(&[], &home);
    assert!(
        out.status.success(),
        "{} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read(&program).unwrap(), new_program);
    let ran = Command::new(&program).output().unwrap();
    assert!(String::from_utf8_lossy(&ran.stdout).contains("updated-program"));
    let _ = std::fs::remove_dir_all(&dir);
}
