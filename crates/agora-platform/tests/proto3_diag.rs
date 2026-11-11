//! PROTOTYPE diagnostics (not for merge): what a process inside the per-agent AppContainer sees for `stat` of the folders above its
//! grant, and on its console handles. Needs `AGORA_PROTO3_PROBE` = path of the probe.exe built in the scratch folder.
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use agora_platform::{spawn, PtyConfig, Sandbox};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn setup(name: &str) -> (PathBuf, PathBuf) {
    std::env::set_var("AGORA_WINDOWS_CONFINEMENT", "1");
    let base = PathBuf::from(std::env::var("AGORA_PROTO3_BASE").expect("AGORA_PROTO3_BASE"));
    let allowed = base.join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&allowed);
    std::fs::create_dir_all(&allowed).unwrap();
    let probe = PathBuf::from(std::env::var("AGORA_PROTO3_PROBE").expect("AGORA_PROTO3_PROBE"));
    let copy = allowed.join("probe.exe");
    std::fs::copy(&probe, &copy).unwrap();
    (allowed, copy)
}

fn run_argv(argv: Vec<String>, allowed: &Path, env: &[(&str, &str)]) -> String {
    let mut cfg = PtyConfig::new(argv);
    cfg.cwd = Some(allowed.to_path_buf());
    for (k, v) in env {
        cfg.env.push(((*k).into(), (*v).into()));
    }
    cfg.sandbox = Some(Sandbox {
        read: vec![],
        write: vec![allowed.to_path_buf()],
    });
    let pty = spawn(&cfg).unwrap();
    let mut out = Vec::new();
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        match pty.output().recv_timeout(Duration::from_millis(100)) {
            Ok(b) => out.extend(b),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(_) => {}
        }
    }
    drop(pty);
    // Keep only printable text and line breaks so the report is readable.
    String::from_utf8_lossy(&out)
        .replace("\u{1b}[", "<CSI>")
        .chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .collect()
}

/// Starts `argv` itself as the confined PTY process and returns its exit status and output.
fn exit_of(argv: Vec<String>, allowed: &Path) -> String {
    let mut cfg = PtyConfig::new(argv);
    cfg.cwd = Some(allowed.to_path_buf());
    cfg.sandbox = Some(Sandbox {
        read: vec![],
        write: vec![allowed.to_path_buf()],
    });
    let mut pty = spawn(&cfg).unwrap();
    let mut out = Vec::new();
    let end = Instant::now() + Duration::from_secs(15);
    let mut status = None;
    while Instant::now() < end {
        if let Ok(b) = pty.output().recv_timeout(Duration::from_millis(100)) {
            out.extend(b);
        }
        if let Ok(Some(s)) = pty.try_wait() {
            status = Some(s);
            break;
        }
    }
    let text: String = String::from_utf8_lossy(&out)
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    format!(
        "status={status:?} printed_git_version={}",
        text.contains("git version")
    )
}

#[test]
#[ignore = "prototype diagnostics: needs AGORA_PROTO3_BASE and AGORA_PROTO3_PROBE"]
fn proto3_git_as_the_pty_process() {
    let (allowed, _probe) = setup("gitdirect");
    let git = "C:\\Program Files\\Git\\ucrt64\\bin\\git.exe".to_string();
    println!(
        "PROTO3-GITDIRECT NOSTRICT={} {}",
        std::env::var("AGORA_PROTO_NOSTRICT").unwrap_or_default(),
        exit_of(vec![git, "--version".into()], &allowed)
    );
    let _ = std::fs::remove_dir_all(&allowed);
}

#[test]
#[ignore = "prototype diagnostics: needs AGORA_PROTO3_BASE and AGORA_PROTO3_PROBE"]
fn proto3_stat_of_the_folders_above_the_grant() {
    let (allowed, probe) = setup("stat");
    let mut argv = vec![probe.display().to_string(), "stat".into()];
    for a in allowed.ancestors() {
        argv.push(a.display().to_string());
    }
    for extra in ["C:\\", "C:\\Users", "C:\\Windows", "C:\\Program Files\\Git"] {
        argv.push(extra.into());
    }
    let out = run_argv(argv, &allowed, &[]);
    println!("PROTO3-STAT\n{out}");
    let _ = std::fs::remove_dir_all(&allowed);
}

#[test]
#[ignore = "prototype diagnostics: needs AGORA_PROTO3_BASE and AGORA_PROTO3_PROBE"]
fn proto3_console_handles_and_git() {
    let (allowed, probe) = setup("console");
    let p = probe.display().to_string();
    println!(
        "PROTO3-CONSOLE-DIRECT\n{}",
        run_argv(vec![p.clone(), "mitigation".into()], &allowed, &[])
    );
    let script = allowed.join("run.cmd");
    let git_mingw = "C:\\Program Files\\Git\\ucrt64\\bin\\git.exe";
    let git_cmd = "C:\\Program Files\\Git\\cmd\\git.exe";
    std::fs::write(
        &script,
        format!(
            "@echo off\r\n\
             \"{p}\" console\r\n\
             echo PROBE-UNDER-CMD-EL=%errorlevel%\r\n\
             \"{p}\" mitigation\r\n\
             \"{p}\" badclose\r\n\
             echo BADCLOSE-EL=%errorlevel%\r\n\
             \"{git_cmd}\" --version\r\n\
             echo GIT-CMD-EL=%errorlevel%\r\n\
             \"{git_mingw}\" --version\r\n\
             echo GIT-MINGW-EL=%errorlevel%\r\n\
             \"{git_mingw}\" --version 2>nul\r\n\
             echo GIT-MINGW-NOERR-EL=%errorlevel%\r\n\
             \"{git_mingw}\" --version > out.txt\r\n\
             echo GIT-MINGW-REDIR-EL=%errorlevel%\r\n\
             \"{git_mingw}\" --version < nul\r\n\
             echo GIT-MINGW-NULIN-EL=%errorlevel%\r\n\
             \"{git_mingw}\" --version 2> err.txt\r\n\
             echo GIT-MINGW-ERRFILE-EL=%errorlevel%\r\n\
             \"{git_mingw}\" --version > out.txt 2>&1 < nul\r\n\
             echo GIT-MINGW-ALLREDIR-EL=%errorlevel%\r\n\
             \"{git_mingw}\" rev-parse --is-inside-work-tree\r\n\
             echo GIT-MINGW-REVPARSE-EL=%errorlevel%\r\n\
             \"C:\\Program Files\\Git\\usr\\bin\\echo.exe\" msys-echo\r\n\
             echo MSYS-ECHO-EL=%errorlevel%\r\n\
             \"{git_mingw}\" --version | findstr git\r\n\
             echo GIT-MINGW-PIPE-EL=%errorlevel%\r\n\
             where.exe cmd\r\n\
             echo WHERE-EL=%errorlevel%\r\n\
             echo x | findstr x\r\n\
             echo FINDSTR-EL=%errorlevel%\r\n"
        ),
    )
    .unwrap();
    let argv = vec![
        "cmd.exe".into(),
        "/c".into(),
        "call".into(),
        script.display().to_string(),
    ];
    println!(
        "PROTO3-CMD-PLAIN\n{}",
        run_argv(argv.clone(), &allowed, &[])
    );
    println!(
        "PROTO3-CMD-ENV\n{}",
        run_argv(
            argv,
            &allowed,
            &[
                ("GIT_CONFIG_NOSYSTEM", "1"),
                ("TERM", "dumb"),
                ("GIT_PAGER", "cat"),
                ("NO_COLOR", "1"),
                ("GIT_TERMINAL_PROMPT", "0"),
            ]
        )
    );
    let _ = std::fs::remove_dir_all(&allowed);
}
