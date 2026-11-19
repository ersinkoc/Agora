//! ADR-0079: the parts of the terminal holder that are the same on every system: the bounded buffer and the signals a node may ask for.
#![allow(dead_code)] // the holder calls these

use std::collections::VecDeque;

/// ADR-0079 decision 4: what the holder keeps of one terminal's output while no node is attached.
pub const HELD_BUFFER: usize = 1 << 20;

/// What a node may ask the holder to send an agent's process group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeldSignal {
    Interrupt,
    Terminate,
    Kill,
    Hangup,
}

// ----- the bounded buffer -----

/// Output kept for a node that is not there: at most `cap` bytes, the oldest dropped first.
#[derive(Debug, Clone)]
pub struct HeldBuffer {
    data: VecDeque<u8>,
    cap: usize,
    dropped: bool,
}

impl Default for HeldBuffer {
    fn default() -> Self {
        Self::with_capacity(HELD_BUFFER)
    }
}

impl HeldBuffer {
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            data: VecDeque::new(),
            cap,
            dropped: false,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        // Only the last `cap` bytes of an oversized write can survive.
        let mut bytes = bytes;
        if bytes.len() > self.cap {
            bytes = &bytes[bytes.len() - self.cap..];
            self.dropped = true;
        }
        let keep = self.cap - bytes.len();
        if self.data.len() > keep {
            let excess = self.data.len() - keep;
            self.data.drain(..excess);
            self.dropped = true;
        }
        self.data.extend(bytes);
    }

    /// Adds what `other` kept after what this one has (the bound applies to the whole).
    pub fn append(&mut self, other: HeldBuffer) {
        let dropped = other.dropped;
        self.push(&other.into_vec());
        self.dropped |= dropped;
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Whether older output was dropped to stay within the bound (what is kept then may start inside an escape sequence).
    pub fn dropped(&self) -> bool {
        self.dropped
    }

    pub fn into_vec(self) -> Vec<u8> {
        Vec::from(self.data)
    }
}

/// What a [`TerminalReader`] kept.
#[derive(Debug)]
pub struct Collected {
    pub buffer: HeldBuffer,
    /// The terminal ended (every process closed its side) while it was read.
    pub ended: bool,
}
