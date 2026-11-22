//! `agora mesh stdio --socket PATH`: the MCP server an agent's client starts, carrying one MCP conversation to that agent's
//! endpoint on the node (`docs/specs/mesh-tools.md`).
//!
//! ADR-0075 decision 6: the node can re-execute or restart while the agent keeps running, so this bridge outlives its connection.
//! When the connection to the agent's socket ends, the bridge
//! - answers every request in flight with `mesh.unavailable` (a JSON-RPC error whose `data` is `{code, params}`), so the agent's
//!   client never waits for an answer that will not come; a request the client cancelled (`notifications/cancelled`) is forgotten
//!   when the cancellation goes out, and gets no answer;
//! - answers requests that arrive while disconnected the same way, at once (a batch's as a batch only where the conversation has
//!   batches, #981), and a line that is not JSON or not a valid request with the JSON-RPC error the node would send (#984);
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

use agora_mesh::mcp::{
    batch_item, cancelled_id, read_line, BatchItem, Line, BATCH_VERSION, INVALID_REQUEST,
    PARSE_ERROR,
};
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
    /// The id of that `initialize` request, as JSON text.
    initialize_id: Option<String>,
    /// The node's answer to it settled on the protocol version that has JSON-RPC batches (2025-03-26, #981).
    batches: bool,
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

    /// Answers a JSON-RPC batch the node will not, reading its values as the node does ([`batch_item`]): each request gets
    /// `mesh.unavailable` when `owed` says it is still this bridge's to answer, notifications get nothing, and in a conversation
    /// with batches (2025-03-26) a value that is not a valid request gets its -32600 and a `notifications/cancelled` in the batch
    /// drops the answer to the request it names, all in one batch. Without batches the client does not read one: each answer is
    /// its own line, and a batch holding no request is one -32600 with id null, as the node would answer it (#981).
    fn refuse_batch(
        &self,
        items: &[serde_json::Value],
        mut owed: impl FnMut(&serde_json::Value) -> bool,
    ) {
        let batches = lock(&self.hello).batches;
        let mut answers: Vec<Message> = Vec::new();
        let mut requests = false;
        for item in items {
            match batch_item(item) {
                BatchItem::Handle {
                    msg: Message::Request { id, .. },
                    ..
                } => {
                    requests = true;
                    if owed(&id) {
                        answers.push(app_error(id, &Error::new(ErrorCode::MeshUnavailable)));
                    }
                }
                BatchItem::Handle { msg, .. } => {
                    if let Some(gone) = cancelled_id(&msg).filter(|_| batches) {
                        answers.retain(
                            |a| !matches!(a, Message::ErrorResponse { id, .. } if id == gone),
                        );
                    }
                }
                BatchItem::Answer(m) => {
                    let has_id = matches!(&m, Message::ErrorResponse { id, .. } if !id.is_null());
                    requests |= has_id;
                    if batches || has_id {
                        answers.push(m);
                    }
                }
                BatchItem::Skip => {}
            }
        }
        if !batches {
            if !requests {
                answers.push(Message::ErrorResponse {
                    id: serde_json::Value::Null,
                    code: INVALID_REQUEST,
                    data: None,
                });
            }
            for answer in &answers {
                self.to_client(to_line(answer).as_bytes());
            }
            return;
        }
        if answers.is_empty() {
            return;
        }
        let answers: Vec<serde_json::Value> = answers
            .iter()
            .filter_map(|a| serde_json::from_str(&to_line(a)).ok())
            .collect();
        self.to_client(serde_json::Value::Array(answers).to_string().as_bytes());
    }

    /// Bookkeeping for a line from the node on connection `generation`: the requests it answers (one, or each in a batch of
    /// answers) are no longer in flight, and the answer to the client's `initialize` says whether the conversation has batches.
    fn answered(&self, generation: u64, line: &[u8]) {
        let ids: Vec<serde_json::Value> = match classify(line) {
            Some(Message::Response { id, result }) => {
                let mut hello = lock(&self.hello);
                if hello.initialize_id.as_deref() == Some(id.to_string().as_str()) {
                    hello.batches = result.get("protocolVersion").and_then(|v| v.as_str())
                        == Some(BATCH_VERSION);
                }
                vec![id]
            }
            Some(Message::ErrorResponse { id, .. }) => vec![id],
            None if line.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'[') => {
                serde_json::from_slice::<Vec<serde_json::Value>>(line)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|v| v.get("id").cloned())
                    .collect()
            }
            _ => Vec::new(),
        };
        if ids.is_empty() {
            return;
        }
        let mut link = lock(&self.link);
        if link.generation == generation {
            for id in ids {
                link.pending.remove(&id.to_string());
            }
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
                shared.answered(generation, &line);
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
    // Not one valid message: a batch, or a line the node answers with a JSON-RPC error (#981, #984). Blank lines are nothing.
    let other: Option<Line> = match (&msg, line.iter().any(|b| !b.is_ascii_whitespace())) {
        (None, true) => Some(match std::str::from_utf8(line) {
            Ok(text) => read_line(text.trim_end()),
            Err(_) => Line::Refuse(Message::ErrorResponse {
                id: serde_json::Value::Null,
                code: PARSE_ERROR,
                data: None,
            }),
        }),
        _ => None,
    };
    let batch: Option<&Vec<serde_json::Value>> = match &other {
        Some(Line::Batch(items)) => Some(items),
        _ => None,
    };
    match &msg {
        Some(Message::Request { id, method, .. }) if method == "initialize" => {
            let mut hello = lock(&shared.hello);
            hello.initialize = Some(line.to_vec());
            hello.initialize_id = Some(id.to_string());
        }
        Some(Message::Notification { method, .. }) if method == "notifications/initialized" => {
            lock(&shared.hello).initialized = Some(line.to_vec());
        }
        _ => {}
    }
    let batches = lock(&shared.hello).batches;
    let (writer, generation, tracked) = {
        let mut link = lock(&shared.link);
        let writer = link.stream.as_ref().and_then(|s| s.try_clone().ok());
        let mut tracked = false;
        if let (Some(_), Some(m @ Message::Request { id, .. })) = (&writer, &msg) {
            link.pending.insert(id.to_string(), m.clone());
            tracked = true;
        }
        // The requests of a batch are in flight too: the node answers each (in an array, or one by one where the conversation
        // has no batches), and the connection's ending answers those it did not. They are not counted in `tracked`, which is only
        // for the single request below. Read in order as the node reads them: where the conversation has batches, a
        // `notifications/cancelled` in the batch drops a request before it, as one on its own line does; where it has none, the
        // node refuses the whole batch and a cancellation in it is inert.
        if let (Some(_), Some(items)) = (&writer, batch) {
            for item in items {
                if let BatchItem::Handle { msg, .. } = batch_item(item) {
                    if let Message::Request { id, .. } = &msg {
                        link.pending.insert(id.to_string(), msg.clone());
                    } else if let Some(id) = cancelled_id(&msg).filter(|_| batches) {
                        link.pending.remove(&id.to_string());
                    }
                }
            }
        }
        // The client gave up on that call: the node sends nothing for it (`docs/specs/mesh-tools.md`), and neither does the
        // connection's ending. Forgotten before the line goes out, so an ending in between cannot answer it.
        if let Some(id) = msg.as_ref().and_then(cancelled_id) {
            link.pending.remove(&id.to_string());
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
    match &other {
        // A batch's requests the ending of the connection already answered are not answered twice: only those still pending on it
        // (or all, when there was no connection) are.
        Some(Line::Batch(items)) => shared.refuse_batch(items, |id| {
            !was_connected || {
                let mut link = lock(&shared.link);
                link.generation == generation && link.pending.remove(&id.to_string()).is_some()
            }
        }),
        // Not JSON, or not a valid request: the JSON-RPC error the node would have sent.
        Some(Line::Refuse(err)) => shared.to_client(to_line(err).as_bytes()),
        _ => {}
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
#[allow(clippy::unwrap_used, clippy::expect_used)]
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

    /// A bridge with no connection to the node, and what it wrote to the client.
    fn disconnected() -> (Shared, Arc<Mutex<Vec<u8>>>) {
        struct Capture(Arc<Mutex<Vec<u8>>>);
        impl Write for Capture {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                lock(&self.0).extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let shared = Shared {
            link: Mutex::new(Link {
                stream: None,
                generation: 0,
                pending: BTreeMap::new(),
            }),
            hello: Mutex::new(Hello::default()),
            out: Mutex::new(Box::new(Capture(Arc::clone(&seen)))),
            closed: AtomicBool::new(false),
        };
        (shared, seen)
    }

    fn written(seen: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8(lock(seen).clone()).unwrap()
    }

    /// JSON-RPC 2.0: a notification never gets a reply, not even an error, so while the node is away only a request is answered
    /// (`mesh.unavailable`).
    #[test]
    fn while_disconnected_a_request_is_answered_and_a_notification_is_not() {
        let (shared, seen) = disconnected();
        to_node(
            &shared,
            br#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#,
        );
        to_node(
            &shared,
            br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        );
        assert_eq!(written(&seen), "", "a notification got a reply");
        to_node(
            &shared,
            br#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{}}"#,
        );
        let out = written(&seen);
        assert_eq!(out.lines().count(), 1, "{out}");
        let reply: serde_json::Value = serde_json::from_str(out.trim_end()).unwrap();
        assert_eq!(reply["id"], 7);
        assert!(reply.get("error").is_some(), "{out}");
        assert!(out.contains("mesh.unavailable"), "{out}");
    }

    /// A cancelled call is the client's to forget (MCP): the node sends nothing for it, and when the connection ends the bridge
    /// must not answer it either. Only the calls still waiting get `mesh.unavailable`.
    #[test]
    fn a_cancelled_call_gets_no_answer_when_the_connection_ends() {
        let path =
            std::env::temp_dir().join(format!("agora-bridge-cancel-{}.sock", std::process::id()));
        let listener = agora_platform::local_listen(&path).unwrap();
        let client = agora_platform::local_connect(&path).unwrap();
        let node = listener.accept().unwrap();
        let (shared, seen) = disconnected();
        lock(&shared.link).stream = Some(client.try_clone().unwrap());
        for line in [
            &br#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"ask"}}"#[..],
            br#"{"jsonrpc":"2.0","id":"s","method":"tools/call","params":{"name":"agent_wait"}}"#,
            br#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"ask"}}"#,
            br#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":5}}"#,
            br#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"s"}}"#,
            // Not a call it sent with this id (a number, not the string "6"): nothing is forgotten.
            br#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"6"}}"#,
        ] {
            to_node(&shared, line);
        }
        // The node got all six lines, the cancellations included.
        let mut got = BufReader::new(node);
        for _ in 0..6 {
            let mut l = String::new();
            got.read_line(&mut l).unwrap();
            assert!(l.ends_with('\n'), "{l:?}");
        }
        assert_eq!(lock(&shared.link).pending.len(), 1);
        shared.lost(0);
        let out = written(&seen);
        assert_eq!(out.lines().count(), 1, "{out}");
        let reply: serde_json::Value = serde_json::from_str(out.trim_end()).unwrap();
        assert_eq!(reply["id"], 6, "{out}");
        assert!(out.contains("mesh.unavailable"), "{out}");
        drop(listener);
        let _ = std::fs::remove_file(&path);
    }

    /// A batch: nothing for one made only of notifications, and one error per request (none for its notifications) otherwise.
    #[test]
    fn while_disconnected_a_batch_of_notifications_gets_no_reply() {
        let (shared, seen) = disconnected();
        lock(&shared.hello).batches = true;
        to_node(
            &shared,
            br#"[{"jsonrpc":"2.0","method":"notifications/cancelled"},{"jsonrpc":"2.0","method":"notifications/cancelled"}]"#,
        );
        assert_eq!(written(&seen), "");
        to_node(
            &shared,
            br#"[{"jsonrpc":"2.0","method":"notifications/cancelled"},{"jsonrpc":"2.0","id":3,"method":"tools/list"}]"#,
        );
        let out = written(&seen);
        let reply: serde_json::Value = serde_json::from_str(out.trim_end()).unwrap();
        let items = reply.as_array().unwrap();
        assert_eq!(items.len(), 1, "{out}");
        assert_eq!(items[0]["id"], 3);
    }

    /// #984: while the node is away the bridge answers what the node would: a line that is not JSON is -32700 with id null, and
    /// one that is not a valid request is -32600 (with its id when one can be read); an empty batch is -32600; a blank line and
    /// a response from the client get nothing.
    #[test]
    fn while_disconnected_a_line_that_is_not_a_request_gets_the_json_rpc_error() {
        let (shared, seen) = disconnected();
        for (line, id, code) in [
            (
                &b"{\"jsonrpc\":\"2.0\",\"id\":4,\"method\""[..],
                serde_json::Value::Null,
                -32700,
            ),
            (b"\xff\xfe\n", serde_json::Value::Null, -32700),
            (
                br#"{"jsonrpc":"1.0","id":5,"method":"ping"}"#,
                serde_json::json!(5),
                -32600,
            ),
            (b"[]", serde_json::Value::Null, -32600),
        ] {
            lock(&seen).clear();
            to_node(&shared, line);
            let out = written(&seen);
            assert_eq!(out.lines().count(), 1, "{out}");
            let reply: serde_json::Value = serde_json::from_str(out.trim_end()).unwrap();
            assert_eq!(reply["id"], id, "{out}");
            assert_eq!(reply["error"]["code"], code, "{out}");
        }
        lock(&seen).clear();
        to_node(&shared, b"   \n");
        to_node(&shared, br#"{"jsonrpc":"2.0","id":9,"result":{}}"#);
        assert_eq!(written(&seen), "");
    }

    /// #981: a client whose conversation has no batches (2025-06-18) does not read one, so while the node is away the requests of a
    /// batch it sent are each answered on their own line.
    #[test]
    fn while_disconnected_a_batch_without_batching_gets_one_line_per_request() {
        let (shared, seen) = disconnected();
        to_node(
            &shared,
            br#"[{"jsonrpc":"2.0","id":3,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/progress"},{"jsonrpc":"2.0","id":4,"method":"tools/list"}]"#,
        );
        let out = written(&seen);
        let ids: Vec<serde_json::Value> = out
            .lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["id"].clone())
            .collect();
        assert_eq!(ids, [serde_json::json!(3), serde_json::json!(4)], "{out}");
        assert_eq!(out.matches("mesh.unavailable").count(), 2, "{out}");
        // Only notifications: one -32600 with id null, as the node answers such a batch, so the line is not met with silence.
        lock(&seen).clear();
        to_node(
            &shared,
            br#"[{"jsonrpc":"2.0","method":"notifications/progress"}]"#,
        );
        let out = written(&seen);
        let reply: serde_json::Value = serde_json::from_str(out.trim_end()).unwrap();
        assert!(
            reply["id"].is_null() && reply["error"]["code"] == -32600,
            "{out}"
        );
    }

    /// #981: the requests of a batch sent to the node are in flight like single ones: the node's array answers them, and when the
    /// connection ends only the one still unanswered gets `mesh.unavailable`. The answer to `initialize` says whether the
    /// conversation has batches.
    #[test]
    fn a_batch_in_flight_is_answered_when_the_connection_ends() {
        let path =
            std::env::temp_dir().join(format!("agora-bridge-batch-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = agora_platform::local_listen(&path).unwrap();
        let client = agora_platform::local_connect(&path).unwrap();
        let node = listener.accept().unwrap();
        let (shared, seen) = disconnected();
        lock(&shared.link).stream = Some(client.try_clone().unwrap());
        to_node(
            &shared,
            br#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
        );
        shared.answered(
            0,
            br#"{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":"2025-03-26"}}"#,
        );
        assert!(lock(&shared.hello).batches);
        to_node(
            &shared,
            br#"[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/progress"},{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ask"}}]"#,
        );
        let mut got = BufReader::new(node.try_clone().unwrap());
        let mut l = String::new();
        for _ in 0..2 {
            l.clear();
            got.read_line(&mut l).unwrap();
        }
        assert!(l.starts_with('['), "{l:?}");
        assert_eq!(lock(&shared.link).pending.len(), 2);
        shared.answered(0, br#"[{"jsonrpc":"2.0","id":1,"result":{}}]"#);
        assert_eq!(lock(&shared.link).pending.len(), 1);
        shared.lost(0);
        let out = written(&seen);
        assert_eq!(out.lines().count(), 1, "{out}");
        let reply: serde_json::Value = serde_json::from_str(out.trim_end()).unwrap();
        assert_eq!(reply["id"], 2, "{out}");
        assert!(out.contains("mesh.unavailable"), "{out}");
        drop(listener);
        let _ = std::fs::remove_file(&path);
    }

    /// A bridge connected to a test node socket at `path`; the node's end is returned so it stays open.
    fn connected(name: &str) -> (Shared, Arc<Mutex<Vec<u8>>>, LocalStream, PathBuf) {
        let path =
            std::env::temp_dir().join(format!("agora-bridge-{name}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = agora_platform::local_listen(&path).unwrap();
        let client = agora_platform::local_connect(&path).unwrap();
        let node = listener.accept().unwrap();
        let (shared, seen) = disconnected();
        lock(&shared.link).stream = Some(client);
        drop(listener);
        (shared, seen, node, path)
    }

    /// Codex review of #992: in a 2025-03-26 conversation the node handles a batch's values in order, so a request cancelled by a
    /// `notifications/cancelled` later in the same batch gets nothing from the node, and must get nothing from the bridge when the
    /// connection ends. Where the conversation has no batches the node refuses the whole batch, so a cancellation in it is inert.
    #[test]
    fn a_request_cancelled_inside_its_batch_gets_no_answer_when_the_connection_ends() {
        let (shared, seen, node, path) = connected("cancel-batch");
        lock(&shared.hello).batches = true;
        to_node(
            &shared,
            br#"[{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"ask"}},{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7}},{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"ask"}}]"#,
        );
        shared.lost(0);
        let out = written(&seen);
        assert_eq!(out.lines().count(), 1, "{out}");
        let reply: serde_json::Value = serde_json::from_str(out.trim_end()).unwrap();
        assert_eq!(reply["id"], 8, "{out}");
        drop(node);

        let (shared, seen, node, _) = connected("cancel-refused");
        to_node(
            &shared,
            br#"[{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"ask"}},{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7}}]"#,
        );
        shared.lost(0);
        let out = written(&seen);
        let reply: serde_json::Value = serde_json::from_str(out.trim_end()).unwrap();
        assert_eq!(reply["id"], 7, "{out}");
        drop(node);
        let _ = std::fs::remove_file(&path);
    }

    /// Codex review of #992: while disconnected in a 2025-03-26 conversation, a batch is answered as the node answers one: each
    /// value that is not a valid request gets -32600 in the array, each request `mesh.unavailable`, notifications nothing, and a
    /// request the batch itself cancels nothing.
    #[test]
    fn while_disconnected_invalid_values_of_a_batch_get_errors_in_the_array() {
        let (shared, seen) = disconnected();
        lock(&shared.hello).batches = true;
        to_node(&shared, br#"[1,{"foo":1}]"#);
        let out = written(&seen);
        let reply: serde_json::Value = serde_json::from_str(out.trim_end()).unwrap();
        let items = reply
            .as_array()
            .unwrap_or_else(|| panic!("not an array: {out}"));
        assert_eq!(items.len(), 2, "{out}");
        assert!(
            items
                .iter()
                .all(|i| i["id"].is_null() && i["error"]["code"] == -32600),
            "{out}"
        );
        lock(&seen).clear();
        to_node(
            &shared,
            br#"[{"jsonrpc":"2.0","id":3,"method":"ping"},"x",{"jsonrpc":"2.0","method":"notifications/progress"},{"jsonrpc":"2.0","id":5,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":5}}]"#,
        );
        let out = written(&seen);
        let reply: serde_json::Value = serde_json::from_str(out.trim_end()).unwrap();
        let items = reply
            .as_array()
            .unwrap_or_else(|| panic!("not an array: {out}"));
        assert_eq!(items.len(), 2, "{out}");
        assert_eq!(items[0]["id"], 3, "{out}");
        assert!(unavailable_reply(&items[0]), "{out}");
        assert_eq!(
            (items[1]["id"].clone(), items[1]["error"]["code"].clone()),
            (serde_json::Value::Null, serde_json::json!(-32600)),
            "{out}"
        );
    }

    fn unavailable_reply(v: &serde_json::Value) -> bool {
        v["error"]["data"]["code"] == "mesh.unavailable"
    }
}
