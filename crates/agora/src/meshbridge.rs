//! `agora mesh stdio --socket PATH`: the MCP server an agent's client starts, carrying one MCP conversation to that agent's
//! endpoint on the node (`docs/specs/mesh-tools.md`).
//!
//! ADR-0075 decision 6: the node can re-execute or restart while the agent keeps running, so this bridge outlives its connection.
//! When the connection to the agent's socket ends, the bridge
//! - answers every request in flight with `mesh.unavailable` (a JSON-RPC error whose `data` is `{code, params}`), so the agent's
//!   client never waits for an answer that will not come;
//! - answers requests that arrive while disconnected the same way, at once;
//! - reconnects to the SAME socket path with bounded exponential backoff, without closing its stdout or exiting, so the client's
//!   session stays open;
//! - on a new connection does what the client did at the start: it replays the client's own `initialize` request (and its
//!   `notifications/initialized`) and swallows the answer, because the node's new connection starts uninitialized. Nothing the
//!   old connection had is carried over: the node decides afresh, and the socket's own checks (owner-only endpoint, the
//!   per-agent identity on Windows) apply to the new connection exactly as to the first.
//!
//! Bound: the bridge keeps retrying while its client keeps stdin open, with the delay doubling from [`FIRST_DELAY`] to
//! [`MAX_DELAY`], and gives up (exits 1, which the client sees as its server ending) after [`MAX_OUTAGE`] without a connection. The
//! endpoint is removed when the agent ends, so a bridge whose agent is gone ends with its client's stdin, not by retrying forever.
//!
//! What crosses is untouched: lines from the node go to the client byte for byte. The bridge never interprets, caches or
//! answers tool content, only request ids.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use agora_platform::LocalStream;
use agora_protocol::jsonrpc::{app_error, parse_line, to_line, Message};
use agora_protocol::{Error, ErrorCode};

/// The first wait before a new connection attempt.
pub const FIRST_DELAY: Duration = Duration::from_millis(50);
/// The longest wait between attempts.
pub const MAX_DELAY: Duration = Duration::from_secs(2);
/// How long the bridge goes without a connection before it gives up.
pub const MAX_OUTAGE: Duration = Duration::from_secs(15 * 60);
/// How long a new connection may take to answer the replayed `initialize`.
const HANDSHAKE_WAIT: Duration = Duration::from_secs(5);
/// Sleeps are cut into slices of this length so the end of the client's input is noticed.
const SLICE: Duration = Duration::from_millis(25);
/// The most a peer may send as the answer to the replayed `initialize` before it is refused.
const HANDSHAKE_MAX: usize = 1 << 20;

/// The waits between connection attempts: doubling, capped.
pub struct Backoff {
    next: Duration,
    cap: Duration,
}

impl Backoff {
    pub fn new(first: Duration, cap: Duration) -> Self {
        Self { next: first, cap }
    }

    /// The wait before the next attempt; the one after is twice as long, never more than the cap.
    pub fn wait(&mut self) -> Duration {
        let now = self.next;
        self.next = (self.next * 2).min(self.cap);
        now
    }
}

/// Whether an outage that began `elapsed` ago has lasted past the bound.
pub fn outage_over(elapsed: Duration, bound: Duration) -> bool {
    elapsed >= bound
}

/// What the connection state holds. One lock, so "pending" and "connected" always agree.
struct Link {
    stream: Option<LocalStream>,
    /// Counts connections, so a late failure of an old one is not mistaken for the current one.
    generation: u64,
    /// Requests sent on the current connection and not yet answered, by their id as JSON text. Each value is the request itself,
    /// which carries the id to answer it with.
    pending: BTreeMap<String, Message>,
}

/// What the client said that a new connection must hear again.
#[derive(Default)]
struct Hello {
    /// The client's `initialize` request line, as it sent it.
    initialize: Option<Vec<u8>>,
    /// Its `notifications/initialized` line.
    initialized: Option<Vec<u8>>,
}

struct Shared {
    link: Mutex<Link>,
    hello: Mutex<Hello>,
    /// Lines to the client. One lock keeps each line whole.
    out: Mutex<Box<dyn Write + Send>>,
    /// The client closed its input: stop reconnecting and end.
    closed: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    /// Writes one line to the client. When the client's output is gone nothing is left to serve, so the bridge ends.
    fn to_client(&self, line: &[u8]) {
        let mut out = lock(&self.out);
        let sent = out
            .write_all(line)
            .and_then(|()| {
                if line.ends_with(b"\n") {
                    Ok(())
                } else {
                    out.write_all(b"\n")
                }
            })
            .and_then(|()| out.flush())
            .is_ok();
        if !sent {
            self.closed.store(true, Ordering::Release);
            std::process::exit(0);
        }
    }

    /// Answers a request the node will not: `mesh.unavailable`, a stable code the client's agent can read.
    fn refuse(&self, request: &Message) {
        if let Message::Request { id, .. } = request {
            let err = app_error(id.clone(), &Error::new(ErrorCode::MeshUnavailable));
            self.to_client(to_line(&err).as_bytes());
        }
    }

    /// Answers a JSON-RPC batch the node will not: a batch of one error per request in it, and nothing when it holds only
    /// notifications (JSON-RPC 2.0, section 6).
    fn refuse_batch(&self, items: &[serde_json::Value]) {
        let errors: Vec<serde_json::Value> = items
            .iter()
            .filter_map(|item| match parse_line(&item.to_string()) {
                Ok(Message::Request { id, .. }) => {
                    let err = app_error(id, &Error::new(ErrorCode::MeshUnavailable));
                    serde_json::from_str(&to_line(&err)).ok()
                }
                _ => None,
            })
            .collect();
        if !errors.is_empty() {
            self.to_client(serde_json::Value::Array(errors).to_string().as_bytes());
        }
    }

    /// The connection `generation` ended: everything in flight on it is answered with an error and the endpoint is shut.
    fn lost(&self, generation: u64) {
        let failed = {
            let mut link = lock(&self.link);
            if link.generation != generation || link.stream.is_none() {
                return;
            }
            if let Some(s) = link.stream.take() {
                let _ = s.shutdown();
            }
            std::mem::take(&mut link.pending)
        };
        for request in failed.values() {
            self.refuse(request);
        }
    }
}

/// Connects to `path` and does what the client did at the start. `Err` is any failure (or the client ending): the caller waits
/// and tries again. The bytes the peer sent after its answer are returned with the stream, so none is lost. The answer is waited for
/// at most [`HANDSHAKE_WAIT`] and read up to [`HANDSHAKE_MAX`], however the peer sends it, and the wait ends when the client does.
fn connect(
    path: &Path,
    hello: &Mutex<Hello>,
    closed: &AtomicBool,
) -> std::io::Result<(LocalStream, Vec<u8>)> {
    let mut stream = agora_platform::local_connect(path)?;
    let (init, ready) = {
        let h = lock(hello);
        (h.initialize.clone(), h.initialized.clone())
    };
    let mut rest = Vec::new();
    if let Some(init) = init {
        stream.set_read_timeout(Some(SLICE))?;
        stream.write_all(&init)?;
        stream.flush()?;
        // The answer belongs to a request the client already had answered: swallow it. Anything but a result means this is not
        // a node that took the handshake, so it is not trusted with the client's calls.
        let begun = Instant::now();
        let mut got = Vec::new();
        let mut chunk = [0u8; 4096];
        let newline = loop {
            if let Some(i) = got.iter().position(|b| *b == b'\n') {
                break i;
            }
            if closed.load(Ordering::Acquire)
                || begun.elapsed() >= HANDSHAKE_WAIT
                || got.len() > HANDSHAKE_MAX
            {
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            match stream.read(&mut chunk) {
                Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => got.extend_from_slice(&chunk[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => return Err(e),
            }
        };
        let ok = std::str::from_utf8(&got[..newline])
            .ok()
            .and_then(|t| parse_line(t.trim_end()).ok())
            .is_some_and(|m| matches!(m, Message::Response { .. }));
        if !ok {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        rest = got.split_off(newline + 1);
        if let Some(ready) = ready {
            stream.write_all(&ready)?;
            stream.flush()?;
        }
        stream.set_read_timeout(None)?;
    }
    Ok((stream, rest))
}

/// The reader of one bridge: lines from the node to the client, and the reconnecting when the node's end closes.
fn read_node(
    shared: &Arc<Shared>,
    path: &Path,
    mut stream: LocalStream,
    mut generation: u64,
    mut rest: Vec<u8>,
) {
    loop {
        if let Ok(clone) = stream.try_clone() {
            let mut reader = BufReader::new(Cursor::new(std::mem::take(&mut rest)).chain(clone));
            let mut line = Vec::new();
            loop {
                line.clear();
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                if let Some(Message::Response { id, .. } | Message::ErrorResponse { id, .. }) =
                    std::str::from_utf8(&line)
                        .ok()
                        .and_then(|t| parse_line(t.trim_end()).ok())
                {
                    let mut link = lock(&shared.link);
                    if link.generation == generation {
                        link.pending.remove(&id.to_string());
                    }
                }
                shared.to_client(&line);
            }
        }
        shared.lost(generation);
        drop(stream);
        match reconnect(shared, path) {
            Some((s, g, r)) => {
                stream = s;
                generation = g;
                rest = r;
            }
            None => return,
        }
    }
}

/// Tries until it connects, the client ends, or the outage passes its bound.
fn reconnect(shared: &Arc<Shared>, path: &Path) -> Option<(LocalStream, u64, Vec<u8>)> {
    let began = Instant::now();
    let mut backoff = Backoff::new(FIRST_DELAY, MAX_DELAY);
    loop {
        let mut left = backoff.wait();
        while !left.is_zero() {
            if shared.closed.load(Ordering::Acquire) {
                return None;
            }
            let slice = left.min(SLICE);
            std::thread::sleep(slice);
            left -= slice;
        }
        if shared.closed.load(Ordering::Acquire) {
            return None;
        }
        if let Ok((stream, rest)) = connect(path, &shared.hello, &shared.closed) {
            let Ok(writer) = stream.try_clone() else {
                continue;
            };
            let mut link = lock(&shared.link);
            // The client may have ended while the handshake ran: `run` shuts the connection it finds, so one installed after that
            // would never be shut.
            if shared.closed.load(Ordering::Acquire) {
                let _ = stream.shutdown();
                return None;
            }
            link.generation += 1;
            link.pending.clear();
            link.stream = Some(writer);
            return Some((stream, link.generation, rest));
        }
        if outage_over(began.elapsed(), MAX_OUTAGE) {
            // The client sees its server end; nothing here can serve it any more.
            shared.closed.store(true, Ordering::Release);
            std::process::exit(1);
        }
    }
}

/// What the client's line is, for the bridge's bookkeeping.
fn classify(line: &[u8]) -> Option<Message> {
    std::str::from_utf8(line)
        .ok()
        .and_then(|t| parse_line(t.trim_end()).ok())
}

/// Sends the client's line to the node, or answers it with `mesh.unavailable` when there is no connection.
fn to_node(shared: &Shared, line: &[u8]) {
    let msg = classify(line);
    let batch: Option<Vec<serde_json::Value>> =
        match (&msg, line.iter().find(|b| !b.is_ascii_whitespace())) {
            (None, Some(b'[')) => serde_json::from_slice(line).ok(),
            _ => None,
        };
    match &msg {
        Some(Message::Request { method, .. }) if method == "initialize" => {
            lock(&shared.hello).initialize = Some(line.to_vec());
        }
        Some(Message::Notification { method, .. }) if method == "notifications/initialized" => {
            lock(&shared.hello).initialized = Some(line.to_vec());
        }
        _ => {}
    }
    let (writer, generation, tracked) = {
        let mut link = lock(&shared.link);
        let writer = link.stream.as_ref().and_then(|s| s.try_clone().ok());
        let mut tracked = false;
        if let (Some(_), Some(m @ Message::Request { id, .. })) = (&writer, &msg) {
            link.pending.insert(id.to_string(), m.clone());
            tracked = true;
        }
        (writer, link.generation, tracked)
    };
    let was_connected = writer.is_some();
    let sent = match writer {
        Some(mut w) => {
            let mut framed = line.to_vec();
            if !framed.ends_with(b"\n") {
                framed.push(b'\n');
            }
            w.write_all(&framed).and_then(|()| w.flush()).is_ok()
        }
        None => false,
    };
    if sent {
        return;
    }
    // Not delivered. A request is answered here, unless the connection's ending already answered it: that answers every request
    // it still held, and a request this call tracked is one of them while the connection is the same.
    if let Some(m @ Message::Request { id, .. }) = &msg {
        let mine = !tracked || {
            let mut link = lock(&shared.link);
            link.generation == generation && link.pending.remove(&id.to_string()).is_some()
        };
        if mine {
            shared.refuse(m);
        }
    }
    if let Some(items) = &batch {
        shared.refuse_batch(items);
    }
    if was_connected {
        // A failed write means the connection written to is finished: wake its reader so it reconnects. A newer connection is not
        // touched.
        let link = lock(&shared.link);
        if link.generation == generation {
            if let Some(s) = link.stream.as_ref() {
                let _ = s.shutdown();
            }
        }
    }
}

/// Runs the bridge over `input` and `output` against the socket at `path`, whose first connection is `first`.
fn run(
    first: LocalStream,
    path: &Path,
    input: impl Read,
    output: Box<dyn Write + Send>,
) -> ExitCode {
    let Ok(writer) = first.try_clone() else {
        return ExitCode::from(1);
    };
    let shared = Arc::new(Shared {
        link: Mutex::new(Link {
            stream: Some(writer),
            generation: 0,
            pending: BTreeMap::new(),
        }),
        hello: Mutex::new(Hello::default()),
        out: Mutex::new(output),
        closed: AtomicBool::new(false),
    });
    let reader = {
        let (shared, path): (Arc<Shared>, PathBuf) = (Arc::clone(&shared), path.to_path_buf());
        std::thread::spawn(move || read_node(&shared, &path, first, 0, Vec::new()))
    };
    let mut input = BufReader::new(input);
    let mut line = Vec::new();
    loop {
        line.clear();
        match input.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if shared.closed.load(Ordering::Acquire) {
            break;
        }
        to_node(&shared, &line);
    }
    // The client hung up: stop reconnecting, close the connection so the reader wakes, and end.
    {
        let mut link = lock(&shared.link);
        shared.closed.store(true, Ordering::Release);
        if let Some(s) = link.stream.take() {
            let _ = s.shutdown();
        }
    }
    let _ = reader.join();
    ExitCode::SUCCESS
}

/// `agora mesh stdio`: `Err` is the failure of the first connection, which the caller reports as for any command.
pub fn mesh_stdio(path: &Path) -> Result<ExitCode, std::io::Error> {
    let first = agora_platform::local_connect(path)?;
    Ok(run(
        first,
        path,
        std::io::stdin().lock(),
        Box::new(std::io::stdout()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_waits_double_and_stop_at_the_cap() {
        let mut b = Backoff::new(Duration::from_millis(50), Duration::from_secs(2));
        let waits: Vec<u128> = (0..10).map(|_| b.wait().as_millis()).collect();
        assert_eq!(
            waits,
            [50, 100, 200, 400, 800, 1600, 2000, 2000, 2000, 2000]
        );
    }

    #[test]
    fn an_outage_ends_at_its_bound_and_not_before() {
        assert!(!outage_over(
            MAX_OUTAGE - Duration::from_millis(1),
            MAX_OUTAGE
        ));
        assert!(outage_over(MAX_OUTAGE, MAX_OUTAGE));
    }
}
