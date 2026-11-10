//! A confined process (ADR-0047) can write where it was allowed and nowhere else, and cannot read what was not listed.
//! Skipped, with a line saying so, on a machine that cannot confine (no Landlock).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use agora_platform::{sandbox_supported, spawn, PtyConfig, Sandbox};
use std::time::{Duration, Instant};

/// Runs `script` in `sh` under `sandbox`; returns everything it printed.
fn run(sandbox: Option<Sandbox>, script: &str) -> String {
    let mut cfg = PtyConfig::new(vec!["/bin/sh".into(), "-c".into(), script.into()]);
    cfg.sandbox = sandbox;
    let pty = spawn(&cfg).unwrap();
    let mut out = Vec::new();
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        match pty.output().recv_timeout(Duration::from_millis(100)) {
            Ok(b) => out.extend(b),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(_) => {}
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[test]
fn a_confined_process_writes_only_where_it_was_allowed() {
    if !sandbox_supported() {
        eprintln!("skipped: no Landlock on this machine");
        return;
    }
    let dir = std::env::temp_dir().join(format!("agora-sandbox-{}", std::process::id()));
    let (allowed, denied, secret) = (dir.join("work"), dir.join("elsewhere"), dir.join("secret"));
    for d in [&allowed, &denied, &secret] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(secret.join("key"), "hunter2").unwrap();
    let sb = Sandbox {
        read: ["/usr", "/lib", "/lib64", "/bin", "/etc", "/dev", "/proc"]
            .map(Into::into)
            .to_vec(),
        write: vec![allowed.clone(), "/dev".into()],
    };
    let script = format!(
        "echo in > {a}/ok && echo wrote-allowed; echo out > {d}/no 2>/dev/null || echo refused-write; cat {s}/key 2>/dev/null || echo refused-read",
        a = allowed.display(),
        d = denied.display(),
        s = secret.display()
    );
    let out = run(Some(sb), &script);
    assert!(out.contains("wrote-allowed"), "{out}");
    assert!(
        out.contains("refused-write") && out.contains("refused-read"),
        "{out}"
    );
    assert!(!out.contains("hunter2"));
    assert!(allowed.join("ok").exists() && !denied.join("no").exists());

    // The same script with no sandbox is not refused anything.
    let out = run(None, &script);
    assert!(out.contains("hunter2") && !out.contains("refused"), "{out}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn asking_for_a_sandbox_that_cannot_be_enforced_is_an_error_not_a_free_pass() {
    if sandbox_supported() {
        return;
    }
    let mut cfg = PtyConfig::new(vec!["/bin/sh".into(), "-c".into(), "true".into()]);
    cfg.sandbox = Some(Sandbox::default());
    assert_eq!(
        spawn(&cfg).err().map(|e| e.kind()),
        Some(std::io::ErrorKind::Unsupported)
    );
}
