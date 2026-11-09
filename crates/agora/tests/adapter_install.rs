//! `agora adapter install|list|remove` against a stand-in registry on loopback, with real Ed25519 signatures (ADR-0044): a package
//! is installed only when the index, the package and the user's answer all agree, and every refusal leaves the folders as they were.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn key() -> Ed25519KeyPair {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
}

fn tar(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data, mode) in files {
        let mut h = [0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..107].copy_from_slice(format!("{mode:07o}").as_bytes());
        h[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
        h[156] = b'0';
        out.extend_from_slice(&h);
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.extend_from_slice(&[0u8; 1024]);
    out
}

fn serve(files: HashMap<String, Vec<u8>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let files = Arc::new(files);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let mut buf = [0u8; 2048];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
            let reply: Vec<u8> = match files.get(&path) {
                Some(body) => {
                    let mut r = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    r.extend_from_slice(body);
                    r
                }
                None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_vec(),
            };
            let _ = s.write_all(&reply);
        }
    });
    port
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref())
}

fn agora(home: &Path, args: &[&str], stdin: &str) -> (bool, String, String) {
    let mut c = Command::new(AGORA)
        .args(args)
        .env("AGORA_HOME", home)
        .env("LC_ALL", "en")
        .env("LANG", "en")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let o = c.wait_with_output().unwrap();
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

struct Registry {
    base: String,
    root_pub: String,
}

/// A registry with one publisher `acme` and one package `demo` 1.0.0 whose tar is `package` (signed by `acme`, or not).
fn registry(package: &[u8], listed_hash: &str, revoked: bool) -> Registry {
    let (root, acme) = (key(), key());
    let payload = serde_json_string(&acme, listed_hash, revoked);
    let sig = hex(root.sign(payload.as_bytes()).as_ref());
    let envelope = format!("{{\"payload\":{},\"sig\":\"{sig}\"}}", json_quote(&payload));
    let mut files = HashMap::new();
    files.insert("/index.json".to_string(), envelope.into_bytes());
    files.insert("/packages/demo-1.0.0.tar".to_string(), package.to_vec());
    files.insert(
        "/packages/demo-1.0.0.sig".to_string(),
        hex(acme.sign(package).as_ref()).into_bytes(),
    );
    let port = serve(files);
    Registry {
        base: format!("http://127.0.0.1:{port}"),
        root_pub: hex(root.public_key().as_ref()),
    }
}

fn json_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn serde_json_string(acme: &Ed25519KeyPair, hash: &str, revoked: bool) -> String {
    format!(
        "{{\"version\":3,\"expires_ms\":{},\"publishers\":[{{\"id\":\"acme\",\"key\":\"{}\",\"revoked\":{revoked}}}],\"packages\":[{{\"name\":\"demo\",\"version\":\"1.0.0\",\"publisher\":\"acme\",\"sha256\":\"{hash}\"}}]}}",
        u64::MAX / 4,
        hex(acme.public_key().as_ref())
    )
}

fn package() -> Vec<u8> {
    tar(&[
        (
            "manifest.toml",
            b"kind = \"demo\"\ndisplay_name = \"Demo\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"demo\"]\n[adapter]\ncommand = [\"./run\", \"--flag\"]\n",
            0o644,
        ),
        ("run", b"#!/bin/sh\nexit 0\n", 0o755),
    ])
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agora-adapter-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn a_signed_package_is_installed_after_the_users_yes_and_listed_and_removed() {
    if !agora_platform::PTY_SUPPORTED {
        return;
    }
    let dir = scratch("ok");
    let (home, manifests) = (dir.join("home"), dir.join("manifests"));
    let pkg = package();
    let r = registry(&pkg, &sha256_hex(&pkg), false);
    let m = manifests.to_string_lossy().into_owned();
    let args = [
        "adapter",
        "install",
        "demo",
        "--registry",
        &r.base,
        "--root-key",
        "",
        "--manifests",
        &m,
    ];
    std::fs::create_dir_all(&home).unwrap();
    let keyfile = dir.join("root.key");
    std::fs::write(&keyfile, &r.root_pub).unwrap();
    let kf = keyfile.to_string_lossy().into_owned();
    let mut a = args.to_vec();
    a[6] = &kf;

    // A "no" installs nothing.
    let (ok, out, _) = agora(&home, &a, "n\n");
    assert!(
        !ok && out.contains("acme") && out.contains("./run --flag"),
        "{out}"
    );
    assert!(!manifests.join("demo.toml").exists() && !home.join("adapters").exists());

    // A "yes" installs it, with the command pointing into the install folder.
    let (ok, out, err) = agora(&home, &a, "y\n");
    assert!(ok, "{out}{err}");
    let installed = std::fs::read_to_string(manifests.join("demo.toml")).unwrap();
    let run = home.join("adapters/demo/1.0.0/run");
    assert!(installed.contains(&*run.to_string_lossy()), "{installed}");
    assert!(run.exists() && home.join("publishers").exists());
    assert_eq!(
        std::fs::read_to_string(home.join("registry-version"))
            .unwrap()
            .trim(),
        "3"
    );

    // The publisher is known now: `--yes` goes through without a question.
    let mut with_yes = a.to_vec();
    with_yes.push("--yes");
    let (ok, out, err) = agora(&home, &with_yes, "");
    assert!(ok, "{out}{err}");

    let (ok, out, _) = agora(&home, &["adapter", "list"], "");
    assert!(
        ok && out.contains("demo 1.0.0") && out.contains("acme") && !out.contains("revoked"),
        "{out}"
    );
    let (ok, _, _) = agora(&home, &["adapter", "remove", "demo", "--manifests", &m], "");
    assert!(ok && !manifests.join("demo.toml").exists() && !home.join("adapters/demo").exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_package_that_does_not_match_its_index_or_its_key_changes_nothing() {
    if !agora_platform::PTY_SUPPORTED {
        return;
    }
    let dir = scratch("bad");
    let (home, manifests) = (dir.join("home"), dir.join("manifests"));
    std::fs::create_dir_all(&home).unwrap();
    let m = manifests.to_string_lossy().into_owned();
    let pkg = package();

    // The index lists another hash than the package has.
    let r = registry(&pkg, &sha256_hex(b"something else"), false);
    let keyfile = dir.join("root.key");
    std::fs::write(&keyfile, &r.root_pub).unwrap();
    let kf = keyfile.to_string_lossy().into_owned();
    let (ok, _, err) = agora(
        &home,
        &[
            "adapter",
            "install",
            "demo",
            "--registry",
            &r.base,
            "--root-key",
            &kf,
            "--manifests",
            &m,
        ],
        "y\n",
    );
    assert!(!ok && err.contains("does not match the index"), "{err}");

    // A different root key than the one that signed the index.
    let r2 = registry(&pkg, &sha256_hex(&pkg), false);
    std::fs::write(&keyfile, hex(key().public_key().as_ref())).unwrap();
    let (ok, _, err) = agora(
        &home,
        &[
            "adapter",
            "install",
            "demo",
            "--registry",
            &r2.base,
            "--root-key",
            &kf,
            "--manifests",
            &m,
        ],
        "y\n",
    );
    assert!(!ok && err.contains("signature"), "{err}");

    // A revoked publisher.
    let r3 = registry(&pkg, &sha256_hex(&pkg), true);
    std::fs::write(&keyfile, &r3.root_pub).unwrap();
    let (ok, _, err) = agora(
        &home,
        &[
            "adapter",
            "install",
            "demo",
            "--registry",
            &r3.base,
            "--root-key",
            &kf,
            "--manifests",
            &m,
        ],
        "y\n",
    );
    assert!(!ok && err.contains("revoked"), "{err}");

    // A registry that is not https and not on this machine is not even asked.
    let (ok, _, err) = agora(
        &home,
        &[
            "adapter",
            "install",
            "demo",
            "--registry",
            "http://example.com",
            "--root-key",
            &kf,
            "--manifests",
            &m,
        ],
        "y\n",
    );
    assert!(!ok && err.contains("https"), "{err}");

    assert!(!manifests.join("demo.toml").exists() && !home.join("adapters").exists());
    let _ = std::fs::remove_dir_all(dir);
}
