//! A confined process (ADR-0047) can write where it was allowed and nowhere else, reads what was listed, and cannot read what
//! was not listed.
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
        eprintln!("skipped: this machine cannot confine");
        return;
    }
    let dir = std::env::temp_dir().join(format!("agora-sandbox-{}", std::process::id()));
    let (allowed, denied, secret, listed) = (
        dir.join("work"),
        dir.join("elsewhere"),
        dir.join("secret"),
        dir.join("listed"),
    );
    for d in [&allowed, &denied, &secret, &listed] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(secret.join("key"), "hunter2").unwrap();
    std::fs::write(listed.join("note"), "listed-note").unwrap();
    let mut read: Vec<std::path::PathBuf> =
        ["/usr", "/lib", "/lib64", "/bin", "/etc", "/dev", "/proc"]
            .map(Into::into)
            .to_vec();
    read.push(listed.clone());
    let sb = Sandbox {
        read,
        write: vec![allowed.clone(), "/dev".into()],
        ..Default::default()
    };
    // The folders are below the temp folder, which macOS keeps private unless listed: the allowed ones must still be readable.
    let script = format!(
        "echo in > {a}/ok && cat {a}/ok && echo wrote-allowed; cat {l}/note; echo out > {d}/no 2>/dev/null || echo refused-write; cat {s}/key 2>/dev/null || echo refused-read",
        a = allowed.display(),
        l = listed.display(),
        d = denied.display(),
        s = secret.display()
    );
    let out = run(Some(sb), &script);
    assert!(out.contains("wrote-allowed"), "{out}");
    assert!(out.contains("listed-note"), "{out}");
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

#[cfg(target_os = "linux")]
#[test]
fn an_unopenable_git_config_leaves_the_launch_environment_alone() {
    use std::os::unix::fs::PermissionsExt;

    if !sandbox_supported() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("agora-unreadable-git-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("git-config");
    std::fs::write(&file, "[user]\nname = Ada Example\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&file).is_ok() {
        // A process with CAP_DAC_OVERRIDE cannot reproduce this open failure.
        let _ = std::fs::remove_dir_all(dir);
        return;
    }
    let sb = Sandbox {
        read: ["/usr", "/lib", "/lib64", "/bin"].map(Into::into).to_vec(),
        write: vec!["/dev".into()],
        git_config: Some(file),
        ..Default::default()
    };

    let mut cmd = std::process::Command::new("/bin/sh");
    assert!(agora_platform::confine_command(&mut cmd, &sb)
        .unwrap()
        .is_none());

    let script = "printf 'git-config=%s\\n' \"$GIT_CONFIG_GLOBAL\"";
    let mut cfg = PtyConfig::new(vec!["/bin/sh".into(), "-c".into(), script.into()]);
    cfg.sandbox = Some(sb);
    cfg.env
        .push(("GIT_CONFIG_GLOBAL".into(), "original".into()));
    let pty = spawn(&cfg).unwrap();
    let mut bytes = Vec::new();
    while let Ok(chunk) = pty.output().recv_timeout(Duration::from_secs(2)) {
        bytes.extend(chunk);
    }
    let out = String::from_utf8_lossy(&bytes);
    assert!(out.contains("git-config=original"), "{out}");
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

/// Landlock rights newer than its first version (F260, F261): `truncate(2)` outside `write` is refused (ABI 3), and a file moves
/// between folders inside `write` while one from a read-only folder cannot be linked in (ABI 2). Linux only: Landlock versions.
#[cfg(target_os = "linux")]
#[test]
fn a_confined_process_cannot_truncate_outside_write_and_can_move_files_inside_it() {
    let python = std::path::Path::new("/usr/bin/python3");
    // SAFETY: the Landlock version query: null attribute, size 0, LANDLOCK_CREATE_RULESET_VERSION.
    let abi = unsafe { libc::syscall(444, std::ptr::null::<u8>(), 0usize, 1u32) };
    if !sandbox_supported() || abi < 3 || !python.exists() {
        eprintln!("skipped: needs Landlock ABI 3 (found {abi}) and /usr/bin/python3");
        return;
    }
    let dir = std::env::temp_dir().join(format!("agora-sandbox-rights-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (work, ro, outside) = (dir.join("work"), dir.join("ro"), dir.join("outside"));
    for d in [work.join("a"), work.join("b"), ro.clone(), outside.clone()] {
        std::fs::create_dir_all(d).unwrap();
    }
    for f in [
        outside.join("victim"),
        ro.join("keep"),
        work.join("inside"),
        work.join("a/f"),
    ] {
        std::fs::write(f, "precious-data").unwrap();
    }
    let script = dir.join("rights.py");
    std::fs::write(
        &script,
        format!(
            "import os\n\
             def t(tag, f):\n\
             \x20   try:\n\
             \x20       f(); print(tag + '=ok')\n\
             \x20   except OSError:\n\
             \x20       print(tag + '=refused')\n\
             t('trunc-outside', lambda: os.truncate('{o}/victim', 0))\n\
             t('trunc-inside', lambda: os.truncate('{w}/inside', 0))\n\
             t('move-inside', lambda: os.rename('{w}/a/f', '{w}/b/f'))\n\
             t('link-readonly', lambda: os.link('{r}/keep', '{w}/stolen'))\n",
            o = outside.display(),
            w = work.display(),
            r = ro.display()
        ),
    )
    .unwrap();
    let mut read: Vec<std::path::PathBuf> =
        ["/usr", "/lib", "/lib64", "/bin", "/etc", "/dev", "/proc"]
            .map(Into::into)
            .to_vec();
    read.extend([ro.clone(), script.clone()]);
    let sb = Sandbox::new(read, vec![work.clone(), "/dev".into()]);
    let out = run(
        Some(sb),
        &format!("{} {}", python.display(), script.display()),
    );
    for want in [
        "trunc-outside=refused",
        "trunc-inside=ok",
        "move-inside=ok",
        "link-readonly=refused",
    ] {
        assert!(out.contains(want), "{want}: {out}");
    }
    assert_eq!(
        std::fs::read_to_string(outside.join("victim")).unwrap(),
        "precious-data"
    );
    assert!(work.join("b/f").exists() && !work.join("stolen").exists());
    let _ = std::fs::remove_dir_all(dir);
}

/// ADR-0083 on Linux: with `confine_sockets`, a confined process cannot connect to an abstract unix socket made outside its sandbox
/// (Landlock ABI 6, Linux 6.12), the same process without it can, and what the kernel cannot restrict (pathname sockets) is not
/// claimed: `socket_confinement` says `AbstractOnly` and a pathname socket stays reachable.
#[cfg(target_os = "linux")]
#[test]
fn a_confined_process_cannot_connect_to_an_abstract_socket_outside_its_sandbox() {
    use agora_platform::{socket_confinement, SocketConfinement};
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixListener};
    let python = std::path::Path::new("/usr/bin/python3");
    if !sandbox_supported() || !python.exists() {
        eprintln!("skipped: needs Landlock and /usr/bin/python3");
        return;
    }
    if socket_confinement() != SocketConfinement::AbstractOnly {
        eprintln!("skipped: this kernel has no Landlock scopes (needs ABI 6)");
        return;
    }
    let name = format!("agora-abstract-{}", std::process::id());
    let abstract_listener =
        UnixListener::bind_addr(&SocketAddr::from_abstract_name(name.as_bytes()).unwrap()).unwrap();
    let dir = std::env::temp_dir().join(format!("agora-sock-scope-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path_listener = UnixListener::bind(dir.join("agent.1")).unwrap();
    let script = format!(
        "import socket\n\
         def go(a):\n\
        \x20   s = socket.socket(socket.AF_UNIX)\n\
        \x20   try:\n\
        \x20       s.connect(a); return 'open'\n\
        \x20   except OSError:\n\
        \x20       return 'closed'\n\
         print('abstract-' + go('\\0{name}'))\n\
         print('pathname-' + go('{path}'))\n",
        path = dir.join("agent.1").display()
    );
    let file = dir.join("probe.py");
    std::fs::write(&file, script).unwrap();
    let read: Vec<std::path::PathBuf> = ["/usr", "/lib", "/lib64", "/bin", "/etc", "/dev", "/proc"]
        .map(Into::into)
        .into_iter()
        .chain([dir.clone()])
        .collect();
    let mut sb = Sandbox {
        read,
        write: vec!["/dev".into()],
        confine_sockets: true,
        ..Default::default()
    };
    let probe = format!("/usr/bin/python3 {}", file.display());
    let out = run(Some(sb.clone()), &probe);
    assert!(out.contains("abstract-closed"), "{out}");
    // Honest about the limit: the pathname socket is still there for the confined process.
    assert!(out.contains("pathname-open"), "{out}");
    // Negative control: without the restriction the abstract socket is reachable too.
    sb.confine_sockets = false;
    let out = run(Some(sb), &probe);
    assert!(out.contains("abstract-open"), "{out}");
    drop((abstract_listener, path_listener));
    let _ = std::fs::remove_dir_all(dir);
}
