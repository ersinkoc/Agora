//! The only crate allowed to contain OS-specific code: PTY/ConPTY, process tree, cwd, local sockets and pipes, resource limits.
//!
//! The crate root exposes platform-neutral types and traits; each OS implements them in `src/<os>.rs`.
//! No `#[cfg(target_os)]` or `#[cfg(unix)]` is allowed outside this crate (AGENTS.md).

use std::io;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

#[cfg(unix)]
mod unix;

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
