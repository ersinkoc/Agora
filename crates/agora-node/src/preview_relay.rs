//! ADR-0088 build step 2b: one relayed connection of a preview forward (`docs/specs/protocol.md`, "Preview").
//!
//! A `PreviewConnect` on a dynamic stream becomes one TCP connection to the forward's port on the loopback, and the stream carries
//! its bytes both ways in `PREVIEW_DATA` frames under credit: each side starts with `window` bytes it may send, and the receiver
//! grants more with `PREVIEW_CREDIT` as it drains. A frame over `chunk`, data beyond the credit, or a byte rate or forward total
//! over its budget ends the connection with `PREVIEW_END` and its reason, then `STREAM_CLOSE`. Nothing in the bytes is read or
//! changed: `Host`, `Origin` and every header go to the program as the browser sent them.
//!
//! Two threads per connection: one copies the program's bytes to the stream as far as the window allows, one writes the window's
//! bytes to the program and grants credit back. Neither buffers more than the window.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use agora_protocol::frame::{kinds, STREAM_CLOSE};
use agora_protocol::v1::{self, PreviewCloseReason as Reason};
use prost::Message;

use crate::preview::{Budgets, Lease};

/// How often each thread looks at the forward, the node and the local connection while it waits.
const POLL: Duration = Duration::from_millis(200);
/// How long a connection to the program may take, on each loopback address.
const CONNECT: Duration = Duration::from_secs(1);

/// Sends one frame on the connection's stream (kind, payload). The control socket's writer, shared with everything else on it.
pub type Send = Arc<dyn Fn(u8, Vec<u8>) -> std::io::Result<()> + std::marker::Send + Sync>;

/// What the window sent on the stream and has not been written to the program yet, with the credit both ways. Registered for the
/// stream before the answer to `PreviewConnect` goes out, so no frame that follows the answer can be missed.
pub struct Inbox {
    st: Mutex<In>,
    cv: Condvar,
    chunk: u64,
}

struct In {
    buf: VecDeque<u8>,
    /// Bytes the window may still send before it is granted more.
    may_receive: u64,
    /// Bytes this side may still send to the window.
    may_send: u64,
    /// The window ended the connection (`PREVIEW_END` or `STREAM_CLOSE`), or the local connection went.
    peer_done: bool,
    /// The window broke the stream's rules.
    violation: bool,
    /// The connection is over; frames that still come are dropped.
    finished: bool,
}

impl Inbox {
    pub fn new(b: &Budgets) -> Self {
        Self {
            st: Mutex::new(In {
                buf: VecDeque::new(),
                may_receive: u64::from(b.window),
                may_send: u64::from(b.window),
                peer_done: false,
                violation: false,
                finished: false,
            }),
            cv: Condvar::new(),
            chunk: u64::from(b.chunk),
        }
    }

    fn st(&self) -> std::sync::MutexGuard<'_, In> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A frame the window sent on this stream: data (kept, within the credit), credit, or its end.
    pub fn push(&self, kind: u8, payload: &[u8]) {
        let mut st = self.st();
        if st.finished {
            return;
        }
        match kind {
            kinds::PREVIEW_DATA => {
                let n = payload.len() as u64;
                if n > self.chunk || n > st.may_receive {
                    st.violation = true;
                } else {
                    st.may_receive -= n;
                    st.buf.extend(payload);
                }
            }
            kinds::PREVIEW_CREDIT => match v1::PreviewCredit::decode(payload) {
                Ok(c) => st.may_send = st.may_send.saturating_add(u64::from(c.bytes)),
                Err(_) => st.violation = true,
            },
            kinds::PREVIEW_END | STREAM_CLOSE => st.peer_done = true,
            _ => {}
        }
        drop(st);
        self.cv.notify_all();
    }

    /// The local connection that carried the stream went: the connection ends without a word.
    pub fn gone(&self) {
        self.st().peer_done = true;
        self.cv.notify_all();
    }

    /// Whether the connection is over (its stream may carry a new `PreviewConnect`).
    pub fn finished(&self) -> bool {
        self.st().finished
    }

    fn finish(&self) {
        self.st().finished = true;
        self.cv.notify_all();
    }
}

/// One relayed connection, shared by its two threads.
struct Conn {
    inbox: Arc<Inbox>,
    send: Send,
    lease: Lease,
    /// The local connection that asked for it is still there.
    alive: Arc<AtomicBool>,
    /// The node is stopping.
    stop: Arc<AtomicBool>,
    ended: AtomicBool,
    to_target: AtomicU64,
    from_target: AtomicU64,
    tcp: TcpStream,
    rate: Mutex<(Instant, u64)>,
}

impl Conn {
    fn budgets(&self) -> Budgets {
        self.lease.budgets
    }

    /// Why the connection must end now from outside it: the forward ended, the node stops, or the local connection went (`None`
    /// inside: nothing is said then).
    fn outside(&self) -> Option<Option<Reason>> {
        if let Some(r) = self.lease.live.ended() {
            return Some(Some(r));
        }
        if self.stop.load(Ordering::Acquire) {
            return Some(Some(Reason::NodeStopped));
        }
        if !self.alive.load(Ordering::Acquire) {
            return Some(None);
        }
        None
    }

    /// Ends the connection once: with a reason, `PREVIEW_END` (the bytes both ways) then `STREAM_CLOSE`; without one, quietly.
    fn end(&self, reason: Option<Reason>) {
        if self.ended.swap(true, Ordering::AcqRel) {
            return;
        }
        self.inbox.finish();
        let _ = self.tcp.shutdown(Shutdown::Both);
        if let Some(reason) = reason {
            let end = v1::PreviewEnd {
                reason: reason as i32,
                bytes_to_target: self.to_target.load(Ordering::Acquire),
                bytes_from_target: self.from_target.load(Ordering::Acquire),
            };
            let _ = (self.send)(kinds::PREVIEW_END, end.encode_to_vec());
            let _ = (self.send)(STREAM_CLOSE, Vec::new());
        }
    }

    fn is_ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }

    /// Counts `n` bytes against the forward's total and this connection's rate; whether both still hold. `wait`: this side may
    /// wait for the next second instead of going over the rate (bytes it reads); the window's bytes cannot wait, so going over ends.
    fn spend(&self, n: u64, wait: bool) -> bool {
        let b = self.budgets();
        if !self.lease.live.count(n, b.total) {
            return false;
        }
        if b.rate == 0 {
            return true;
        }
        let rate = u64::from(b.rate);
        loop {
            let mut r = self.rate.lock().unwrap_or_else(|e| e.into_inner());
            if r.0.elapsed() >= Duration::from_secs(1) {
                *r = (Instant::now(), 0);
            }
            if r.1 + n <= rate {
                r.1 += n;
                return true;
            }
            if !wait || n > rate {
                return false;
            }
            let left = Duration::from_secs(1).saturating_sub(r.0.elapsed());
            drop(r);
            std::thread::sleep(left.min(POLL));
            if self.is_ended() || self.outside().is_some() {
                return true;
            }
        }
    }
}

/// Connects to `port` on the loopback (in the order `localhost` resolves, the two literals only); `None` when nothing answers.
pub fn dial(port: u16) -> Option<TcpStream> {
    crate::preview::loopback_order(port)
        .iter()
        .find_map(|a| TcpStream::connect_timeout(a, CONNECT).ok())
}

/// The answer to a `PreviewConnect` the node accepted.
pub fn connect_result(b: &Budgets) -> v1::PreviewConnectResult {
    v1::PreviewConnectResult {
        max_chunk_bytes: b.chunk,
        window_bytes: b.window,
        rate_bytes_per_second: b.rate,
        total_bytes_limit: b.total,
    }
}

/// Ends a connection over the budget at once (`BUDGET`, no bytes either way), after its answer went out.
pub fn refuse_over_budget(inbox: &Inbox, send: &Send) {
    inbox.finish();
    let end = v1::PreviewEnd {
        reason: Reason::Budget as i32,
        bytes_to_target: 0,
        bytes_from_target: 0,
    };
    let _ = send(kinds::PREVIEW_END, end.encode_to_vec());
    let _ = send(STREAM_CLOSE, Vec::new());
}

/// Relays `tcp` on the stream until either side ends it, the forward ends, the node stops or the local connection goes. Starts the
/// two threads and returns; an error means they could not be started (the connection is then ended quietly).
pub fn start(
    lease: Lease,
    tcp: TcpStream,
    inbox: Arc<Inbox>,
    send: Send,
    alive: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) -> std::io::Result<()> {
    tcp.set_read_timeout(Some(POLL))?;
    tcp.set_nodelay(true)?;
    let reader = tcp.try_clone()?;
    let writer = tcp.try_clone()?;
    let conn = Arc::new(Conn {
        inbox,
        send,
        lease,
        alive,
        stop,
        ended: AtomicBool::new(false),
        to_target: AtomicU64::new(0),
        from_target: AtomicU64::new(0),
        tcp,
        rate: Mutex::new((Instant::now(), 0)),
    });
    let c = Arc::clone(&conn);
    let first = std::thread::Builder::new()
        .name("agora-preview-out".into())
        .spawn(move || from_target(&c, reader));
    if let Err(e) = first {
        conn.end(None);
        return Err(e);
    }
    let c = Arc::clone(&conn);
    std::thread::Builder::new()
        .name("agora-preview-in".into())
        .spawn(move || to_target(&c, writer))
        .map(|_| ())
        .inspect_err(|_| conn.end(None))
}

/// The program's bytes to the window, as far as the credit allows. Every byte read is sent before the end is said.
fn from_target(c: &Conn, mut tcp: TcpStream) {
    let chunk = u64::from(c.budgets().chunk);
    let mut buf = vec![0u8; c.budgets().chunk as usize];
    loop {
        if c.is_ended() {
            return;
        }
        if let Some(r) = c.outside() {
            return c.end(r);
        }
        // Wait for credit.
        let allowed = {
            let mut st = c.inbox.st();
            if st.may_send == 0 && !st.finished {
                st = c
                    .inbox
                    .cv
                    .wait_timeout(st, POLL)
                    .map_or_else(|e| e.into_inner().0, |(g, _)| g);
            }
            st.may_send.min(chunk)
        };
        if allowed == 0 {
            continue;
        }
        let want = usize::try_from(allowed).unwrap_or(buf.len()).min(buf.len());
        match tcp.read(&mut buf[..want]) {
            Ok(0) => return c.end(Some(Reason::TargetGone)),
            Ok(n) => {
                if !c.spend(n as u64, true) {
                    return c.end(Some(Reason::Budget));
                }
                c.inbox.st().may_send -= n as u64;
                if (c.send)(kinds::PREVIEW_DATA, buf[..n].to_vec()).is_err() {
                    return c.end(None);
                }
                c.from_target.fetch_add(n as u64, Ordering::AcqRel);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return c.end(Some(Reason::TargetGone)),
        }
    }
}

/// The window's bytes to the program, granting the credit back as they are written.
fn to_target(c: &Conn, mut tcp: TcpStream) {
    loop {
        if c.is_ended() {
            return;
        }
        if let Some(r) = c.outside() {
            return c.end(r);
        }
        let (bytes, violation, peer_done) = {
            let mut st = c.inbox.st();
            if st.buf.is_empty() && !st.violation && !st.peer_done && !st.finished {
                st = c
                    .inbox
                    .cv
                    .wait_timeout(st, POLL)
                    .map_or_else(|e| e.into_inner().0, |(g, _)| g);
            }
            let bytes: Vec<u8> = st.buf.drain(..).collect();
            (bytes, st.violation, st.peer_done)
        };
        if violation {
            return c.end(Some(Reason::Protocol));
        }
        if !bytes.is_empty() {
            let n = bytes.len() as u64;
            if !c.spend(n, false) {
                return c.end(Some(Reason::Budget));
            }
            if tcp.write_all(&bytes).is_err() {
                return c.end(Some(Reason::TargetGone));
            }
            c.to_target.fetch_add(n, Ordering::AcqRel);
            c.inbox.st().may_receive += n;
            let credit = v1::PreviewCredit {
                bytes: u32::try_from(n).unwrap_or(u32::MAX),
            };
            if (c.send)(kinds::PREVIEW_CREDIT, credit.encode_to_vec()).is_err() {
                return c.end(None);
            }
        }
        if peer_done && bytes.is_empty() {
            // The window ended it: everything it sent was written; nothing is said back.
            return c.end(None);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn small() -> Budgets {
        Budgets {
            chunk: 4,
            window: 8,
            rate: 0,
            total: 0,
        }
    }

    fn violated(i: &Inbox) -> bool {
        i.st().violation
    }

    #[test]
    fn one_byte_frames_up_to_the_window_pass_and_one_more_breaks_the_rules() {
        let i = Inbox::new(&small());
        for _ in 0..8 {
            i.push(kinds::PREVIEW_DATA, b"x");
        }
        assert!(!violated(&i));
        assert_eq!(i.st().buf.len(), 8);
        i.push(kinds::PREVIEW_DATA, b"x");
        assert!(violated(&i), "a byte beyond the credit");
    }

    #[test]
    fn a_frame_over_the_chunk_breaks_the_rules_even_within_the_credit() {
        let i = Inbox::new(&small());
        i.push(kinds::PREVIEW_DATA, b"12345");
        assert!(violated(&i));
        assert!(i.st().buf.is_empty(), "nothing of it is kept");
    }

    #[test]
    fn credit_adds_to_what_this_side_may_send_and_garbage_breaks_the_rules() {
        let i = Inbox::new(&small());
        i.push(
            kinds::PREVIEW_CREDIT,
            &v1::PreviewCredit { bytes: 5 }.encode_to_vec(),
        );
        assert_eq!(i.st().may_send, 13);
        assert!(!violated(&i));
        i.push(kinds::PREVIEW_CREDIT, &[0xff, 0xff, 0xff]);
        assert!(violated(&i));
    }

    #[test]
    fn the_end_from_the_window_and_a_finished_connection() {
        let i = Inbox::new(&small());
        i.push(STREAM_CLOSE, &[]);
        assert!(i.st().peer_done);
        i.finish();
        i.push(kinds::PREVIEW_DATA, b"late");
        assert!(i.st().buf.is_empty(), "frames after the end are dropped");
        assert!(i.finished());
    }
}
