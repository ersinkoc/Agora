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
            ..Default::default()
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
    // Nor a Low label (ADR-0050 Amendment B).
    assert!(!acl.contains("Mandatory Label"), "{acl}");
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
        ..Default::default()
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
            ..Default::default()
        },
        &a,
        &script,
    );
    assert!(out.contains("REFUSED-B"), "{out}");
    assert!(!b.join("from-a.txt").exists());
    drop(agent_b);
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
            ..Default::default()
        },
        &allowed,
        &script,
    );
    assert!(out.contains("REFUSED-VIA"), "{out}");
    assert!(!outside.join("via.txt").exists());
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
            ..Default::default()
        },
        &allowed,
        &script,
    );
    assert!(out.contains("REFUSED-OPEN"), "{out}");
    // Reads are not part of the promise (ADR-0050 Amendment B, 5): a file that grants `Users` or `Everyone` read stays readable.
    assert!(!open.join("no.txt").exists());
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
         echo piped| findstr piped > nul && echo PIPE-WORKS|| echo PIPE-FAILED\r\n\
         git --version && echo GIT-STARTED|| echo GIT-FAILED\r\n",
    )
    .unwrap();
    let out = run(
        Sandbox {
            read: vec![],
            write: vec![allowed.clone()],
            ..Default::default()
        },
        &allowed,
        &script,
    );
    assert!(out.contains("CMD-STARTED"), "{out}");
    // `cmd` opens the pipe it made for `a | b` through the token's default DACL (review of #273, 1).
    assert!(out.contains("PIPE-WORKS"), "{out}");
    assert!(out.contains("GIT-STARTED"), "{out}");
    let _ = std::fs::remove_dir_all(&root);
}

/// Amendment B, 4: an allowed folder that inherits a `Users`/`Everyone` write grant is refused before the agent starts, so a second
/// agent can never reach it through that grant and the Low label.
#[test]
fn an_allowed_folder_with_a_broad_write_grant_is_refused_before_start() {
    enable();
    let root = scratch("broad-refused");
    grant_everyone_and_users(&root);
    let b = root.join("agent-b");
    std::fs::create_dir_all(&b).unwrap();
    let mut cfg = PtyConfig::new(vec!["cmd.exe".into(), "/c".into(), "echo started".into()]);
    cfg.cwd = Some(b.clone());
    cfg.sandbox = Some(Sandbox {
        read: vec![],
        write: vec![b.clone()],
        ..Default::default()
    });
    let err = spawn(&cfg).err().expect("must be refused");
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied, "{err}");
    assert_eq!(
        agora_platform::sandbox_refusal(&err),
        Some(agora_platform::SandboxRefusal::BroadWriteGrant)
    );
    // Nothing was changed: no label, no entry for an agent.
    let acl = acl_text(&b);
    assert!(!acl.contains("Mandatory Label"), "{acl}");
    assert!(
        !acl.lines()
            .any(|l| l.contains("S-1-5-21-") && !l.contains("(I)")),
        "{acl}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Amendment B, 4 (whole tree): one subfolder with an explicit `Everyone` write grant refuses the whole allowed folder.
#[test]
fn a_broad_grant_deep_inside_an_allowed_folder_is_refused() {
    enable();
    let root = scratch("broad-deep");
    let work = root.join("work");
    let deep = work.join("a").join("b");
    std::fs::create_dir_all(&deep).unwrap();
    let out = std::process::Command::new("icacls")
        .arg(&deep)
        .args(["/grant", "*S-1-1-0:(OI)(CI)M"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let mut cfg = PtyConfig::new(vec!["cmd.exe".into(), "/c".into(), "echo started".into()]);
    cfg.cwd = Some(work.clone());
    cfg.sandbox = Some(Sandbox {
        read: vec![],
        write: vec![work.clone()],
        ..Default::default()
    });
    let err = spawn(&cfg).err().expect("must be refused");
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied, "{err}");
    assert_eq!(
        agora_platform::sandbox_refusal(&err),
        Some(agora_platform::SandboxRefusal::BroadWriteGrant)
    );
    let acl = acl_text(&work);
    assert!(!acl.contains("Mandatory Label"), "{acl}");
    let _ = std::fs::remove_dir_all(&root);
}

/// The mesh binds an agent's identity to its connection: under confinement an agent's endpoint admits exactly that agent's
/// identity, so a second confined agent cannot connect to it (ADR-0050 Amendment B on Windows).
#[test]
fn a_confined_agent_cannot_reach_another_agents_endpoint() {
    enable();
    let root = scratch("endpoint");
    let (a, b) = (root.join("agent-a"), root.join("agent-b"));
    for d in [&a, &b] {
        std::fs::create_dir_all(d).unwrap();
    }
    let id_a = agora_platform::new_agent_identity().expect("an identity under confinement");
    let id_b = agora_platform::new_agent_identity().expect("an identity under confinement");
    assert_ne!(id_a, id_b);
    let endpoint = root.join("a.sock");
    let _listener = agora_platform::local_listen_agent(&endpoint, Some(&id_a)).unwrap();
    let pipe = agora_platform::local_endpoint_name(&endpoint).unwrap();
    let try_connect = |dir: &Path, identity: &str, tag: &str| {
        let script = dir.join("connect.cmd");
        std::fs::write(
            &script,
            format!(
                "@echo off\r\necho x> \"{pipe}\" && echo CONNECTED-{tag}|| echo REFUSED-{tag}\r\n"
            ),
        )
        .unwrap();
        let mut cfg = PtyConfig::new(vec![
            "cmd.exe".into(),
            "/c".into(),
            script.display().to_string(),
        ]);
        cfg.cwd = Some(dir.to_path_buf());
        cfg.sandbox = Some(Sandbox {
            read: vec![],
            write: vec![dir.to_path_buf()],
            ..Default::default()
        });
        cfg.identity = Some(identity.to_string());
        let pty = spawn(&cfg).unwrap();
        let mut out = Vec::new();
        let end = Instant::now() + Duration::from_secs(30);
        while Instant::now() < end {
            match pty.output().recv_timeout(Duration::from_millis(100)) {
                Ok(bytes) => out.extend(bytes),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(_) => {}
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    // B first: a refused open leaves the pipe instance free, so A's success below shows B was refused for who it is, not a busy pipe.
    let out_b = try_connect(&b, &id_b, "B");
    assert!(
        out_b.contains("REFUSED-B") && !out_b.contains("CONNECTED-B"),
        "{out_b}"
    );
    let out_a = try_connect(&a, &id_a, "A");
    assert!(out_a.contains("CONNECTED-A"), "{out_a}");
    let _ = std::fs::remove_dir_all(&root);
}

/// The access rights tried on another agent's process, by name.
const PROCESS_RIGHTS: [(&str, u32); 5] = [
    ("QUERY", 0x0400),         // PROCESS_QUERY_INFORMATION
    ("QUERY-LIMITED", 0x1000), // PROCESS_QUERY_LIMITED_INFORMATION
    ("VM-READ", 0x0010),       // PROCESS_VM_READ
    ("TERMINATE", 0x0001),     // PROCESS_TERMINATE
    ("ALL", 0x001F_FFFF),      // PROCESS_ALL_ACCESS
];

/// Opens the process named by `AGORA_TEST_OPEN_PID` with each of [`PROCESS_RIGHTS`] and prints the outcome. Run by
/// `a_confined_agent_cannot_open_another_agents_process` inside a confined agent; on its own it does nothing.
#[test]
#[ignore = "helper run inside a confined agent"]
fn open_process_helper() {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::OpenProcess;
    let Some(pid) = std::env::var("AGORA_TEST_OPEN_PID")
        .ok()
        .and_then(|p| p.parse::<u32>().ok())
    else {
        return;
    };
    for (name, right) in PROCESS_RIGHTS {
        // SAFETY: a plain call; the handle is closed below.
        let h = unsafe { OpenProcess(right, 0, pid) };
        if h.is_null() {
            println!("OPEN-{name}-REFUSED");
        } else {
            println!("OPEN-{name}-GRANTED");
            // SAFETY: opened above, closed once.
            unsafe { CloseHandle(h) };
        }
    }
}

/// Review of #273, 1: a confined agent's process (and threads) take the token's default DACL, which names the user and that agent's
/// own SID only, so a second confined agent cannot open it for anything: no query, no memory read, no terminate.
#[test]
fn a_confined_agent_cannot_open_another_agents_process() {
    enable();
    let root = scratch("open-process");
    let (a, b) = (root.join("agent-a"), root.join("agent-b"));
    for d in [&a, &b] {
        std::fs::create_dir_all(d).unwrap();
    }
    let hold = a.join("hold.cmd");
    std::fs::write(&hold, "@echo off\r\nping -n 30 127.0.0.1 > nul\r\n").unwrap();
    let mut cfg = PtyConfig::new(vec![
        "cmd.exe".into(),
        "/c".into(),
        hold.display().to_string(),
    ]);
    cfg.cwd = Some(a.clone());
    cfg.sandbox = Some(Sandbox {
        read: vec![],
        write: vec![a.clone()],
        ..Default::default()
    });
    let agent_a = spawn(&cfg).unwrap();
    let pid = agent_a.pid();

    // The node itself (not confined) can open it: the pid is right, and a refusal below is about who asks.
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::OpenProcess;
        // SAFETY: a plain call; the handle is closed below.
        let h = unsafe { OpenProcess(0x0400, 0, pid) };
        assert!(!h.is_null(), "the node cannot open agent A ({pid})");
        // SAFETY: opened above, closed once.
        unsafe { CloseHandle(h) };
    }

    let exe = std::env::current_exe().unwrap();
    let mut cfg = PtyConfig::new(vec![
        exe.display().to_string(),
        "--exact".into(),
        "open_process_helper".into(),
        "--ignored".into(),
        "--nocapture".into(),
        "--test-threads=1".into(),
    ]);
    cfg.cwd = Some(b.clone());
    cfg.env
        .push(("AGORA_TEST_OPEN_PID".into(), pid.to_string()));
    cfg.sandbox = Some(Sandbox {
        read: vec![],
        write: vec![b.clone()],
        ..Default::default()
    });
    let agent_b = spawn(&cfg).unwrap();
    let mut out = Vec::new();
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end {
        match agent_b.output().recv_timeout(Duration::from_millis(100)) {
            Ok(bytes) => out.extend(bytes),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(_) => {}
        }
    }
    drop(agent_b);
    drop(agent_a);
    // The permissions go back on the terminal's waiter thread once the killed process is gone (so a node's stop does not wait for the
    // walk): give it a moment, then nothing of agent A is left.
    let clean = |acl: &str| {
        !acl.contains("Mandatory Label")
            && !acl
                .lines()
                .any(|l| l.contains("S-1-5-21-") && !l.contains("(I)"))
    };
    let end = Instant::now() + Duration::from_secs(20);
    let mut acl = acl_text(&a);
    while !clean(&acl) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(100));
        acl = acl_text(&a);
    }
    assert!(clean(&acl), "{acl}");
    let out = String::from_utf8_lossy(&out).into_owned();
    for (name, _) in PROCESS_RIGHTS {
        assert!(
            out.contains(&format!("OPEN-{name}-REFUSED")),
            "{name}: {out}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// From #241 (the #230 negative tests), adapted to ADR-0050 Amendment B: a second agent's folder that inherits `Users` and `Everyone`
/// Modify is refused before that agent starts (`broad_write_grant`), and a confined agent A still cannot write into it.
#[test]
fn a_second_agents_folder_under_a_broad_grant_is_out_of_reach() {
    enable();
    let broad = scratch("broad-two");
    grant_everyone_and_users(&broad);
    let b = broad.join("agent-b");
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(b.join("notes.txt"), "agent-b-private").unwrap();
    let mut cfg = PtyConfig::new(vec!["cmd.exe".into(), "/c".into(), "echo started".into()]);
    cfg.cwd = Some(b.clone());
    cfg.sandbox = Some(Sandbox {
        read: vec![],
        write: vec![b.clone()],
        ..Default::default()
    });
    let err = spawn(&cfg)
        .err()
        .expect("agent B must be refused before start");
    assert_eq!(
        agora_platform::sandbox_refusal(&err),
        Some(agora_platform::SandboxRefusal::BroadWriteGrant),
        "{err}"
    );

    // Agent A lives elsewhere and tries B's folder anyway.
    let root_a = scratch("broad-two-a");
    let a = root_a.join("agent-a");
    std::fs::create_dir_all(&a).unwrap();
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
            ..Default::default()
        },
        &a,
        &script,
    );
    assert!(out.contains("REFUSED-B"), "{out}");
    assert!(!b.join("from-a.txt").exists());
    let _ = std::fs::remove_dir_all(&broad);
    let _ = std::fs::remove_dir_all(&root_a);
}
