//! Windows agent confinement (ADR-0050): a process under a restricted token with a per-agent SID writes only where it was allowed, cannot
//! read what only the user may read, is kept apart from a second agent, passes the rule on to its children, and leaves no folder
//! permission behind. Switched on with `AGORA_WINDOWS_CONFINEMENT=1`, because the feature is off by default.
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use agora_platform::{sandbox_supported, spawn, PtyConfig, Sandbox};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn enable() {
    std::env::set_var("AGORA_WINDOWS_CONFINEMENT", "1");
    assert!(sandbox_supported());
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("agora-winsb-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs the batch file `script` (already inside `allowed`) under `sandbox` and returns what it printed.
fn run(sandbox: Sandbox, allowed: &Path, script: &Path) -> String {
    let mut cfg = PtyConfig::new(vec![
        "cmd.exe".into(),
        "/c".into(),
        script.display().to_string(),
    ]);
    cfg.cwd = Some(allowed.to_path_buf());
    cfg.sandbox = Some(sandbox);
    let pty = spawn(&cfg).unwrap();
    let mut out = Vec::new();
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end {
        match pty.output().recv_timeout(Duration::from_millis(100)) {
            Ok(b) => out.extend(b),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(_) => {}
        }
    }
    drop(pty);
    String::from_utf8_lossy(&out).into_owned()
}

fn acl_text(dir: &Path) -> String {
    let out = std::process::Command::new("icacls")
        .arg(dir)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_confined_process_writes_only_where_it_was_allowed_and_reads_no_user_secret() {
    enable();
    let root = scratch("basic");
    let (allowed, other, secret) = (
        root.join("work"),
        root.join("elsewhere"),
        root.join("secret"),
    );
    for d in [&allowed, &other, &secret] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(secret.join("key"), "hunter2").unwrap();
    let script = allowed.join("run.cmd");
    std::fs::write(
        &script,
        format!(
            "@echo off\r\n\
             echo in> \"{a}\\ok.txt\" && echo WROTE-ALLOWED|| echo REFUSED-ALLOWED\r\n\
             echo out> \"{o}\\no.txt\" && echo WROTE-OTHER|| echo REFUSED-OTHER\r\n\
             type \"{s}\\key\" && echo READ-SECRET|| echo REFUSED-READ\r\n\
             cmd /c \"echo child> {o}\\child.txt\" && echo CHILD-WROTE|| echo CHILD-REFUSED\r\n",
            a = allowed.display(),
            o = other.display(),
            s = secret.display()
        ),
    )
    .unwrap();
    let out = run(
        Sandbox {
            read: vec![],
            write: vec![allowed.clone()],
        },
        &allowed,
        &script,
    );
    assert!(out.contains("WROTE-ALLOWED"), "{out}");
    assert!(out.contains("REFUSED-OTHER"), "{out}");
    assert!(
        out.contains("REFUSED-READ") && !out.contains("hunter2"),
        "{out}"
    );
    assert!(out.contains("CHILD-REFUSED"), "{out}");
    assert!(allowed.join("ok.txt").exists());
    assert!(!other.join("no.txt").exists() && !other.join("child.txt").exists());

    // The entry is gone once the agent has ended.
    // No explicit entry of a per-agent SID is left (inherited ones, `(I)`, belong to other tools and folders above).
    let acl = acl_text(&allowed);
    assert!(
        !acl.lines()
            .any(|l| l.contains("S-1-5-21-") && !l.contains("(I)")),
        "{acl}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_second_agents_folder_is_out_of_reach() {
    enable();
    let root = scratch("two");
    let (a, b) = (root.join("agent-a"), root.join("agent-b"));
    for d in [&a, &b] {
        std::fs::create_dir_all(d).unwrap();
    }
    // Agent B holds its folder through a running process while agent A tries to write into it.
    let hold = b.join("hold.cmd");
    std::fs::write(&hold, "@echo off\r\nping -n 20 127.0.0.1 > nul\r\n").unwrap();
    let mut cfg = PtyConfig::new(vec![
        "cmd.exe".into(),
        "/c".into(),
        hold.display().to_string(),
    ]);
    cfg.cwd = Some(b.clone());
    cfg.sandbox = Some(Sandbox {
        read: vec![],
        write: vec![b.clone()],
    });
    let agent_b = spawn(&cfg).unwrap();

    let script = a.join("run.cmd");
    std::fs::write(
        &script,
        format!(
            "@echo off\r\necho x> \"{b}\\from-a.txt\" && echo WROTE-B|| echo REFUSED-B\r\n",
            b = b.display()
        ),
    )
    .unwrap();
    let out = run(
        Sandbox {
            read: vec![],
            write: vec![a.clone()],
        },
        &a,
        &script,
    );
    assert!(out.contains("REFUSED-B"), "{out}");
    assert!(!b.join("from-a.txt").exists());
    drop(agent_b);
    let _ = std::fs::remove_dir_all(&root);
}

/// Grants `Users` and `Everyone` Modify on `dir`, inherited by everything below it (issue #230: such a grant must not open the folder).
fn grant_everyone_and_users(dir: &Path) {
    for sid in ["*S-1-5-32-545:(OI)(CI)M", "*S-1-1-0:(OI)(CI)M"] {
        let out = std::process::Command::new("icacls")
            .arg(dir)
            .args(["/grant", sid])
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    }
}

#[test]
fn a_folder_every_user_may_write_is_still_refused() {
    enable();
    let root = scratch("broad");
    let (allowed, open) = (root.join("work"), root.join("open-to-all"));
    for d in [&allowed, &open] {
        std::fs::create_dir_all(d).unwrap();
    }
    grant_everyone_and_users(&open);
    std::fs::write(open.join("key"), "hunter2").unwrap();
    let script = allowed.join("run.cmd");
    std::fs::write(
        &script,
        format!(
            "@echo off\r\n\
             echo x> \"{o}\\no.txt\" && echo WROTE-OPEN|| echo REFUSED-OPEN\r\n\
             type \"{o}\\key\" && echo READ-OPEN|| echo REFUSED-READ\r\n",
            o = open.display()
        ),
    )
    .unwrap();
    let out = run(
        Sandbox {
            read: vec![],
            write: vec![allowed.clone()],
        },
        &allowed,
        &script,
    );
    assert!(out.contains("REFUSED-OPEN"), "{out}");
    assert!(
        out.contains("REFUSED-READ") && !out.contains("hunter2"),
        "{out}"
    );
    assert!(!open.join("no.txt").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_second_agents_folder_under_a_broad_grant_is_out_of_reach() {
    enable();
    // Both agents' folders inherit `Users` and `Everyone` Modify from their parent.
    let root = scratch("broad-two");
    grant_everyone_and_users(&root);
    let (a, b) = (root.join("agent-a"), root.join("agent-b"));
    for d in [&a, &b] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(b.join("notes.txt"), "agent-b-private").unwrap();
    let hold = b.join("hold.cmd");
    std::fs::write(&hold, "@echo off\r\nping -n 20 127.0.0.1 > nul\r\n").unwrap();
    let mut cfg = PtyConfig::new(vec![
        "cmd.exe".into(),
        "/c".into(),
        hold.display().to_string(),
    ]);
    cfg.cwd = Some(b.clone());
    cfg.sandbox = Some(Sandbox {
        read: vec![],
        write: vec![b.clone()],
    });
    let agent_b = spawn(&cfg).unwrap();

    let script = a.join("run.cmd");
    std::fs::write(
        &script,
        format!(
            "@echo off\r\n\
             echo x> \"{b}\\from-a.txt\" && echo WROTE-B|| echo REFUSED-B\r\n\
             type \"{b}\\notes.txt\" && echo READ-B|| echo REFUSED-READ-B\r\n",
            b = b.display()
        ),
    )
    .unwrap();
    let out = run(
        Sandbox {
            read: vec![],
            write: vec![a.clone()],
        },
        &a,
        &script,
    );
    assert!(out.contains("REFUSED-B"), "{out}");
    assert!(
        out.contains("REFUSED-READ-B") && !out.contains("agent-b-private"),
        "{out}"
    );
    assert!(!b.join("from-a.txt").exists());
    drop(agent_b);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn system_tools_still_start_when_confined() {
    enable();
    let root = scratch("tools");
    let allowed = root.join("work");
    std::fs::create_dir_all(&allowed).unwrap();
    // `cmd` itself and `git` from Program Files (present on every Windows CI runner).
    let script = allowed.join("run.cmd");
    std::fs::write(
        &script,
        "@echo off\r\n\
         echo CMD-STARTED\r\n\
         git --version && echo GIT-STARTED|| echo GIT-FAILED\r\n",
    )
    .unwrap();
    let out = run(
        Sandbox {
            read: vec![],
            write: vec![allowed.clone()],
        },
        &allowed,
        &script,
    );
    assert!(out.contains("CMD-STARTED"), "{out}");
    assert!(out.contains("GIT-STARTED"), "{out}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_junction_pointing_outside_is_refused() {
    enable();
    let root = scratch("junction");
    let (allowed, outside) = (root.join("work"), root.join("outside"));
    for d in [&allowed, &outside] {
        std::fs::create_dir_all(d).unwrap();
    }
    let link = allowed.join("link");
    let made = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(&outside)
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
    let script = allowed.join("run.cmd");
    std::fs::write(
        &script,
        format!(
            "@echo off\r\necho x> \"{l}\\via.txt\" && echo WROTE-VIA|| echo REFUSED-VIA\r\n",
            l = link.display()
        ),
    )
    .unwrap();
    let out = run(
        Sandbox {
            read: vec![],
            write: vec![allowed.clone()],
        },
        &allowed,
        &script,
    );
    assert!(out.contains("REFUSED-VIA"), "{out}");
    assert!(!outside.join("via.txt").exists());
    let _ = std::fs::remove_dir_all(&root);
}
