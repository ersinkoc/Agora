//! The only crate allowed to contain OS-specific code: PTY/ConPTY, process tree, cwd, local sockets and pipes, resource limits.
//!
//! The crate root exposes platform-neutral types and traits; each OS implements them in `src/<os>.rs`.
//! No `#[cfg(target_os)]` or `#[cfg(unix)]` is allowed outside this crate (AGENTS.md).

use std::io;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

#[cfg(unix)]
mod unix;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
use linux as procinfo;
#[cfg(target_os = "macos")]
use macos as procinfo;

/// How to start a process in a new pseudo-terminal.
#[derive(Debug, Clone)]
pub struct PtyConfig {
    /// Program and arguments; must not be empty.
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Extra environment variables. `TERM` defaults to `xterm-256color` unless set here.
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
}

impl PtyConfig {
    pub fn new(argv: Vec<String>) -> Self {
        Self {
            argv,
            cwd: None,
            env: Vec::new(),
            cols: 80,
            rows: 24,
        }
    }
}

/// How a process ended. Exactly one of the fields is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    /// Exit code when the process exited normally.
    pub code: Option<i32>,
    /// Signal number when it was terminated by a signal (Unix).
    pub signal: Option<i32>,
}

impl ExitStatus {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// A process running in a PTY.
///
/// Output is read on a dedicated OS thread and delivered through a bounded channel in chunks of at most
/// 16 KiB, so a busy terminal cannot starve the async runtime or queue ahead of input (ADR-0013). The channel
/// closes when the process side of the terminal is gone.
pub trait Pty: Send {
    /// Bytes the process wrote to the terminal.
    fn output(&self) -> &Receiver<Vec<u8>>;
    /// Send input to the process.
    fn write_input(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn resize(&mut self, cols: u16, rows: u16) -> io::Result<()>;
    /// Process id of the child (also its process group id).
    fn pid(&self) -> u32;
    /// Kill the child and everything in its process group.
    fn kill(&mut self) -> io::Result<()>;
    /// Non-blocking: `Some` once the child has exited.
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>>;
    /// Block until the child exits.
    fn wait(&mut self) -> io::Result<ExitStatus>;
    /// Process id of the terminal's foreground process group leader (what the user sees running).
    fn foreground_pid(&self) -> io::Result<u32>;
}

/// One process in a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    /// Executable name (the kernel's short name; may be truncated).
    pub name: String,
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "process inspection is not implemented on this platform",
    )
}

/// Short executable name of a process.
pub fn process_name(pid: u32) -> io::Result<String> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        procinfo::name(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        Err(unsupported())
    }
}

/// Current working directory of a process.
pub fn process_cwd(pid: u32) -> io::Result<PathBuf> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        procinfo::cwd(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        Err(unsupported())
    }
}

/// Direct children of a process.
pub fn process_children(pid: u32) -> io::Result<Vec<u32>> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        procinfo::children(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        Err(unsupported())
    }
}

/// The process and all its descendants, parents before children. Processes that exit while the tree is
/// being walked are skipped.
pub fn process_tree(root: u32) -> io::Result<Vec<ProcessInfo>> {
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from([(root, 0u32)]);
    while let Some((pid, ppid)) = queue.pop_front() {
        let Ok(name) = process_name(pid) else {
            if pid == root {
                return Err(io::Error::new(io::ErrorKind::NotFound, "process not found"));
            }
            continue;
        };
        out.push(ProcessInfo { pid, ppid, name });
        if let Ok(kids) = process_children(pid) {
            queue.extend(kids.into_iter().map(|k| (k, pid)));
        }
    }
    Ok(out)
}

/// Start a process in a new PTY.
pub fn spawn(config: &PtyConfig) -> io::Result<Box<dyn Pty>> {
    if config.argv.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "argv is empty"));
    }
    #[cfg(unix)]
    {
        unix::spawn(config)
    }
    #[cfg(not(unix))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "PTY is not implemented on this platform yet",
        ))
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod procinfo_tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if f() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("timed out waiting for {what}");
    }

    #[test]
    fn reports_cwd_foreground_and_tree() {
        let mut p = spawn(&PtyConfig::new(vec!["sh".into()])).expect("spawn");
        p.write_input(b"cd / && sleep 30\n").expect("write");
        let shell = p.pid();

        wait_until("sleep in the foreground", || {
            p.foreground_pid()
                .ok()
                .and_then(|pid| process_name(pid).ok())
                .is_some_and(|n| n == "sleep")
        });
        let fg = p.foreground_pid().expect("fg");
        assert_ne!(fg, shell);
        assert_eq!(process_cwd(fg).expect("cwd"), PathBuf::from("/"));

        let tree = process_tree(shell).expect("tree");
        assert_eq!(tree[0].pid, shell);
        assert!(tree
            .iter()
            .any(|i| i.pid == fg && i.name == "sleep" && i.ppid == shell));

        p.kill().expect("kill");
        p.wait().expect("wait");
    }

    #[test]
    fn missing_process_is_an_error() {
        assert!(process_name(u32::MAX - 1).is_err());
        assert!(process_tree(u32::MAX - 1).is_err());
    }
}
