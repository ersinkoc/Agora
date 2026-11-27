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

/// What a node's terminal of a held agent needs from its holder (ADR-0079): the node reads and writes the terminal itself, and asks the
/// holder, which is the agent's parent, for what only a parent knows. Implemented by the node over its holder link; this crate
/// cannot name the link.
pub trait HeldControl: Send + Sync {
    /// Sends `signal` to the agent's process group, through the holder (which knows whether the leader was collected).
    fn signal(&self, signal: HeldSignal) -> std::io::Result<()>;
    /// How the agent ended: `Ok(None)` while it runs; the status once it ended, taken from the holder (which forgets the agent then).
    /// `Err` when the holder cannot be asked (it ended): the agent's own end is then unknown.
    fn exit_status(&self) -> std::io::Result<Option<crate::ExitStatus>>;
    /// Resizes the agent's terminal through the holder (M2.6: on Windows the pseudo-console stays in the holder, so the node's copy of
    /// the pipe ends cannot resize it). On Unix the node resizes its own copy of the master and never asks this.
    fn resize(&self, cols: u16, rows: u16) -> std::io::Result<()> {
        let _ = (cols, rows);
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }
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

// ----- lines of a pipe-based agent (ADR-0093) -----

/// ADR-0093 decision 1: the longest line the holder keeps of a pipe agent: a whole buffer. A longer line is dropped and counted.
pub const HELD_LINE_MAX: usize = HELD_BUFFER - LINE_COST;

/// What a line costs the bound beyond its bytes: the length the link puts before it. The bound is on the **encoded** size, so a hand-over of
/// everything kept (`seen` and `fresh` together) is never larger than the bound plus a few bytes, whatever the lines' lengths: a flood of
/// one-byte lines cannot make a reply larger than a frame may be.
pub const LINE_COST: usize = 4;

/// The newest lines of a pipe agent's output, at most `cap` bytes (the oldest dropped first, whole lines only, so what is kept always
/// begins at a line). Lines already handed to a node stay until pushed out: a node that restarts is given them again, marked as seen,
/// so what was waiting on the person (ADR-0093 decision 3) can be rebuilt. Bytes that were pushed out before any node saw them are
/// counted (`dropped`), and said once, with the next hand-over.
#[derive(Debug, Clone)]
pub struct HeldLines {
    lines: VecDeque<(u64, Vec<u8>)>,
    bytes: usize,
    cap: usize,
    next_seq: u64,
    /// Every line with a number up to this was handed to a node.
    delivered: u64,
    dropped: u64,
}

impl Default for HeldLines {
    fn default() -> Self {
        Self::with_capacity(HELD_BUFFER)
    }
}

/// What a hand-over carries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineChunk {
    /// Bytes that were dropped before any node saw them since the last hand-over.
    pub dropped: u64,
    /// Lines handed to a node before, oldest first (only in a replay).
    pub seen: Vec<Vec<u8>>,
    /// Lines no node has had, oldest first.
    pub fresh: Vec<Vec<u8>>,
}

impl HeldLines {
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            bytes: 0,
            cap,
            next_seq: 1,
            delivered: 0,
            dropped: 0,
        }
    }

    /// Adds one line (without its newline). An empty line is nothing; one longer than the bound is dropped and counted.
    pub fn push(&mut self, line: Vec<u8>) {
        if line.is_empty() {
            return;
        }
        if line.len() + LINE_COST > self.cap {
            self.dropped += line.len() as u64;
            return;
        }
        while self.bytes + line.len() + LINE_COST > self.cap {
            let Some((seq, old)) = self.lines.pop_front() else {
                break;
            };
            self.bytes -= old.len() + LINE_COST;
            if seq > self.delivered {
                self.dropped += old.len() as u64;
            }
        }
        self.bytes += line.len() + LINE_COST;
        self.lines.push_back((self.next_seq, line));
        self.next_seq += 1;
    }

    /// Counts bytes that could not be kept at all (a line given up).
    pub fn lose(&mut self, bytes: u64) {
        self.dropped += bytes;
    }

    /// The lines no node has had, now handed over.
    pub fn take_fresh(&mut self) -> LineChunk {
        let fresh = self
            .lines
            .iter()
            .filter(|(s, _)| *s > self.delivered)
            .map(|(_, l)| l.clone())
            .collect();
        self.delivered = self.next_seq - 1;
        LineChunk {
            dropped: std::mem::take(&mut self.dropped),
            seen: Vec::new(),
            fresh,
        }
    }

    /// Everything kept, for a node that comes after another: what an earlier node had, and what no node had.
    pub fn replay(&mut self) -> LineChunk {
        let seen = self
            .lines
            .iter()
            .filter(|(s, _)| *s <= self.delivered)
            .map(|(_, l)| l.clone())
            .collect();
        let mut chunk = self.take_fresh();
        chunk.seen = seen;
        chunk
    }

    pub fn len(&self) -> usize {
        self.bytes
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// What the thread reading a pipe agent's output shares with the holder.
#[derive(Debug, Default)]
struct PipeInner {
    lines: HeldLines,
    /// The output ended (every writer closed it).
    eof: bool,
}

/// A pipe agent's output, read by a thread of its own into a [`HeldLines`], so the agent never blocks on a full pipe whether or not a node
/// is there. Cloning shares it.
#[derive(Debug, Clone, Default)]
pub struct PipeLines {
    inner: std::sync::Arc<std::sync::Mutex<PipeInner>>,
}

impl PipeLines {
    /// As [`Default`], with a bound of `cap` bytes (tests use a small one).
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: std::sync::Arc::new(std::sync::Mutex::new(PipeInner {
                lines: HeldLines::with_capacity(cap),
                eof: false,
            })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PipeInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Reads `output` line by line on a thread of its own until it ends. A line that is too long is dropped and counted, never held.
    pub fn spawn_reader<R: std::io::Read + Send + 'static>(
        &self,
        output: R,
    ) -> std::io::Result<std::thread::JoinHandle<()>> {
        let me = self.clone();
        std::thread::Builder::new()
            .name("agora-held-pipe".into())
            .spawn(move || {
                let mut r = std::io::BufReader::new(output);
                let mut line: Vec<u8> = Vec::new();
                let mut over: u64 = 0;
                loop {
                    let chunk = match std::io::BufRead::fill_buf(&mut r) {
                        Ok([]) => break,
                        Ok(c) => c,
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    };
                    let (take, done) = match chunk.iter().position(|b| *b == b'\n') {
                        Some(i) => (i + 1, true),
                        None => (chunk.len(), false),
                    };
                    let content = take - usize::from(done);
                    if over > 0 || line.len() + content > HELD_LINE_MAX {
                        over += content as u64 + line.len() as u64;
                        line.clear();
                    } else {
                        line.extend_from_slice(&chunk[..content]);
                    }
                    std::io::BufRead::consume(&mut r, take);
                    if done {
                        let mut g = me.lock();
                        if over > 0 {
                            g.lines.lose(over);
                        } else {
                            g.lines.push(std::mem::take(&mut line));
                        }
                        line.clear();
                        over = 0;
                    }
                }
                // A last line without its newline is a line too (a writer that ended mid-way keeps what it said).
                let mut g = me.lock();
                if over > 0 {
                    g.lines.lose(over);
                } else if !line.is_empty() {
                    g.lines.push(line);
                }
                g.eof = true;
            })
    }

    /// The lines no node had; and whether the output ended and nothing more will come.
    pub fn take_fresh(&self) -> (LineChunk, bool) {
        let mut g = self.lock();
        let c = g.lines.take_fresh();
        (c, g.eof)
    }

    /// Everything kept (see [`HeldLines::replay`]); and whether the output ended.
    pub fn replay(&self) -> (LineChunk, bool) {
        let mut g = self.lock();
        let c = g.lines.replay();
        (c, g.eof)
    }

    pub fn ended(&self) -> bool {
        self.lock().eof
    }
}

/// Why a line was not queued for a pipe agent's input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueError {
    /// The agent does not read (its queue is full).
    Full,
    /// Its input is closed.
    Closed,
}

/// A pipe agent's input, written by a thread of its own, so a holder never waits on an agent that does not read.
#[derive(Debug)]
pub struct LineWriter {
    tx: std::sync::mpsc::SyncSender<Vec<u8>>,
}

/// Lines that may wait for one agent to read them.
pub const LINE_QUEUE: usize = 256;

impl LineWriter {
    pub fn spawn<W: std::io::Write + Send + 'static>(mut input: W) -> std::io::Result<Self> {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(LINE_QUEUE);
        std::thread::Builder::new()
            .name("agora-held-pipe-in".into())
            .spawn(move || {
                while let Ok(mut line) = rx.recv() {
                    line.push(b'\n');
                    if input.write_all(&line).and_then(|()| input.flush()).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self { tx })
    }

    pub fn queue(&self, line: Vec<u8>) -> Result<(), QueueError> {
        self.tx.try_send(line).map_err(|e| match e {
            std::sync::mpsc::TrySendError::Full(_) => QueueError::Full,
            std::sync::mpsc::TrySendError::Disconnected(_) => QueueError::Closed,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod line_tests {
    use super::*;

    #[test]
    fn the_oldest_whole_lines_go_first_and_unseen_bytes_are_counted_once() {
        let mut l = HeldLines::with_capacity(16);
        for s in ["aaaa", "bbbb"] {
            l.push(s.as_bytes().to_vec());
        }
        l.push(b"cccc".to_vec()); // pushes "aaaa" out unseen
        let c = l.take_fresh();
        assert_eq!(c.dropped, 4);
        assert_eq!(c.fresh, vec![b"bbbb".to_vec(), b"cccc".to_vec()]);
        assert_eq!(l.take_fresh(), LineChunk::default(), "said once");
        // Seen lines pushed out are not a loss.
        l.push(b"dddd".to_vec());
        assert_eq!(l.take_fresh().dropped, 0);
    }

    #[test]
    fn a_replay_has_what_an_earlier_node_had_and_what_no_node_had() {
        let mut l = HeldLines::with_capacity(100);
        l.push(b"one".to_vec());
        assert_eq!(l.take_fresh().fresh.len(), 1);
        l.push(b"two".to_vec());
        let c = l.replay();
        assert_eq!(c.seen, vec![b"one".to_vec()]);
        assert_eq!(c.fresh, vec![b"two".to_vec()]);
        let again = l.replay();
        assert_eq!(again.seen.len(), 2);
        assert!(again.fresh.is_empty());
    }

    #[test]
    fn a_flood_of_tiny_lines_keeps_the_encoded_size_within_the_bound() {
        // One-byte lines cost five each: the kept lines, encoded, never exceed the bound.
        let mut l = HeldLines::with_capacity(100);
        for _ in 0..1000 {
            l.push(b"y".to_vec());
        }
        let c = l.replay();
        let encoded: usize = c.fresh.iter().map(|x| x.len() + LINE_COST).sum();
        assert!(encoded <= 100, "{encoded}");
        assert_eq!(c.fresh.len(), 20);
        assert_eq!(c.dropped, 980);
    }

    #[test]
    fn a_line_longer_than_the_bound_is_dropped_and_counted() {
        let mut l = HeldLines::with_capacity(8);
        l.push(b"toolong".to_vec());
        l.push(Vec::new());
        let c = l.take_fresh();
        assert_eq!((c.dropped, c.fresh.len()), (7, 0));
    }
}
