//! ADR-0079: where the terminal holder is not built yet (Windows has its own primitives, `winhold`; they join this surface in W2),
//! the same names exist and refuse with `Unsupported`, so code that uses the holder compiles on every system.
#![allow(dead_code)]

use std::io;
use std::time::Duration;

use crate::held::Collected;
use crate::held::HeldSignal;
use crate::{ExitStatus, PtyConfig};

fn unsupported() -> io::Error {
    io::Error::from(io::ErrorKind::Unsupported)
}

/// A terminal's master as one passable thing (Unix only for now).
#[derive(Debug)]
pub struct TerminalHandle(());

impl TerminalHandle {
    pub fn into_file(self) -> io::Result<std::fs::File> {
        Err(unsupported())
    }
}

/// A held agent's terminal as a [`crate::Pty`] (Unix only for now).
pub fn held_pty(
    _master: TerminalHandle,
    _pid: u32,
    _control: std::sync::Arc<dyn crate::HeldControl>,
) -> io::Result<Box<dyn crate::Pty>> {
    Err(unsupported())
}

/// A program on a terminal the holder owns (Unix only for now).
#[derive(Debug)]
pub struct HeldTerminal(());

impl HeldTerminal {
    pub fn start(_config: &PtyConfig) -> io::Result<Self> {
        Err(unsupported())
    }
    pub fn pid(&self) -> u32 {
        0
    }
    pub fn kill_and_reap(&mut self) {}
    pub fn resize(&self, _cols: u16, _rows: u16) -> io::Result<()> {
        Err(unsupported())
    }
    pub fn signal(&self, _signal: HeldSignal) -> io::Result<()> {
        Err(unsupported())
    }
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Err(unsupported())
    }
    pub fn handle(&self) -> io::Result<TerminalHandle> {
        Err(unsupported())
    }
    pub fn start_reader(&self) -> io::Result<TerminalReader> {
        Err(unsupported())
    }
    pub fn drain(&self, _buf: &mut crate::held::HeldBuffer) -> io::Result<bool> {
        Err(unsupported())
    }
}

/// A terminal read into a bounded buffer (Unix only for now).
pub struct TerminalReader(());

impl TerminalReader {
    pub fn ended(&self) -> bool {
        false
    }
    pub fn buffered(&self) -> usize {
        0
    }
    pub fn end(self) -> Collected {
        Collected {
            buffer: crate::held::HeldBuffer::default(),
            ended: false,
        }
    }
}

/// Another process (the node, or the holder), watched for its end (ADR-0079 M2.6). On Windows the process is opened once and checked
/// against its start time on that same handle (W1's `OtherProcess`), and every later look and every handle the holder duplicates into
/// it go through that handle, so the check and the use cannot name two processes.
#[cfg(windows)]
pub struct ProcessWatch(crate::winhold::OtherProcess);

#[cfg(windows)]
impl ProcessWatch {
    /// Watches process `pid`, which must have started at `started` ([`crate::process_start_time`]); `NotFound` when no such process
    /// runs or another one has its pid now.
    pub fn open(pid: u32, started: u64) -> io::Result<Self> {
        crate::winhold::OtherProcess::open(pid, started).map(Self)
    }
    pub fn pid(&self) -> u32 {
        self.0.pid()
    }
    /// Whether the process has ended, without waiting.
    pub fn ended(&self) -> bool {
        self.0.ended(Duration::ZERO)
    }
    /// Waits up to `timeout` for the process to end.
    pub fn wait(&self, timeout: Duration) -> bool {
        self.0.ended(timeout)
    }
}

/// Another process watched for its end (not built on this system).
#[cfg(not(windows))]
pub struct ProcessWatch(());

#[cfg(not(windows))]
impl ProcessWatch {
    pub fn open(_pid: u32, _started: u64) -> io::Result<Self> {
        Err(unsupported())
    }
    pub fn pid(&self) -> u32 {
        0
    }
    pub fn ended(&self) -> bool {
        true
    }
    pub fn wait(&self, _timeout: Duration) -> bool {
        true
    }
}

pub(crate) fn close_inherited_descriptors() {}

/// ADR-0079 decision 2 on Windows: nothing to do here. The holder is started detached (no console, a process group of its own) and out
/// of its starter's kill-on-close job ([`crate::spawn_detached`] with `CREATE_BREAKAWAY_FROM_JOB`), which is what a new session gives
/// on Unix.
#[cfg(windows)]
pub(crate) fn start_new_session() -> io::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn start_new_session() -> io::Result<()> {
    Err(unsupported())
}

#[cfg(all(test, windows))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    /// M2.6: the watch the holder keeps on its node, through the public surface: a process that is not there (or not the one that
    /// started then) is `NotFound`, and a watched one's end is seen.
    #[test]
    fn a_watched_process_is_checked_by_its_start_time_and_its_end_is_seen() {
        let mut child = Command::new("ping")
            .args(["-n", "2", "127.0.0.1"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        let started = crate::process_start_time(pid).unwrap();
        assert_eq!(
            ProcessWatch::open(pid, started + 1).err().unwrap().kind(),
            io::ErrorKind::NotFound
        );
        let watch = ProcessWatch::open(pid, started).unwrap();
        assert_eq!(watch.pid(), pid);
        assert!(!watch.ended());
        assert!(!watch.wait(Duration::from_millis(50)));
        assert!(watch.wait(Duration::from_secs(20)));
        assert!(watch.ended());
        let _ = child.wait();
        // Ended: never opened again, even while `watch` still holds the ended process's object (same pid, same start time).
        assert_eq!(
            ProcessWatch::open(pid, started).err().unwrap().kind(),
            io::ErrorKind::NotFound
        );
        drop(watch);
        // A pid no process has: `NotFound`, as on Unix.
        assert_eq!(
            ProcessWatch::open(0xFFFF_FFF0, 1).err().unwrap().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn starting_a_new_session_is_already_done_by_the_detached_start() {
        assert!(start_new_session().is_ok());
    }
}

/// A program on pipes the holder owns (Unix only for now).
#[derive(Debug)]
pub struct HeldProcess(());

impl HeldProcess {
    pub fn start(
        _argv: &[String],
        _cwd: Option<&std::path::Path>,
        _env: &[(String, String)],
    ) -> io::Result<Self> {
        Err(unsupported())
    }
    pub fn take_io(&mut self) -> Option<(std::process::ChildStdin, std::process::ChildStdout)> {
        None
    }
    pub fn pid(&self) -> u32 {
        0
    }
    pub fn signal(&self, _signal: HeldSignal) -> io::Result<()> {
        Err(unsupported())
    }
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Err(unsupported())
    }
    pub fn kill_and_reap(&mut self) {}
}
