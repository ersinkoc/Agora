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

/// Another process watched for its end (Unix only for now).
pub struct ProcessWatch(());

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

pub(crate) fn start_new_session() -> io::Result<()> {
    Err(unsupported())
}
