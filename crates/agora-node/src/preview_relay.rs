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
/// After the window ended a connection, how long what it sent may still take to reach a program that reads slowly; a program that
/// does not read at all does not hold the connection longer.
const DRAIN: Duration = Duration::from_secs(5);

/// Sends one frame on the connection's stream (kind, payload). The control socket's writer, shared with everything else on it.
pub type Send = Arc<dyn Fn(u8, Vec<u8>) -> std::io::Result<()> + std::marker::Send + Sync>;

/// What the window sent on the stream and has not been written to the program yet, with the credit both ways. Registered for the
/// stream before the answer to `PreviewConnect` goes out, so no frame that follows the answer can be missed.
pub struct Inbox {
    st: Mutex<In>,
    cv: Condvar,
    chunk: u64,
    /// `true` once the connection is over. Every frame the connection sends is written while this lock is held and reads `false`,
    /// and the end is written under it too, so no frame of this connection can follow its end or reach a later connection on the
    /// same stream (review of d267e547).
    gate: Mutex<bool>,
    /// The window said the connection is over (`PREVIEW_END`, `STREAM_CLOSE`) or the local connection went: nothing more is sent, and
    /// nothing is said back.
    peer_closed: AtomicBool,
}

struct In {
    buf: VecDeque<u8>,
    /// Bytes the window may still send before it is granted more.
    may_receive: u64,
    /// Bytes this side may still send to the window.
    may_send: u64,
    /// The window ended the connection (`PREVIEW_END` or `STREAM_CLOSE`), or the local connection went.
    peer_done: bool,
    /// When `peer_done` was set: what the window sent is still written to the program for [`DRAIN`], then the connection ends
    /// whatever the program does.
    done_at: Option<Instant>,
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
                done_at: None,
                violation: false,
                finished: false,
            }),
            cv: Condvar::new(),
            chunk: u64::from(b.chunk),
            gate: Mutex::new(false),
            peer_closed: AtomicBool::new(false),
        }
    }

    fn gate(&self) -> std::sync::MutexGuard<'_, bool> {
        self.gate.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Sends one frame of this connection, only while it is not over; whether it went out. `sent` runs under the gate once it did,
    /// so what it counts is in the end's counters exactly when the frame went out before the end.
    fn send_open(&self, send: &Send, kind: u8, payload: Vec<u8>, sent: impl FnOnce()) -> bool {
        let g = self.gate();
        if *g || self.peer_closed.load(Ordering::Acquire) {
            return false;
        }
        let ok = send(kind, payload).is_ok();
        if ok {
            sent();
        }
        drop(g);
        ok
    }

    /// Ends the connection if it is not over yet: `say` runs under the gate (it writes the end), unless the window already ended it.
    /// Whether this call ended it.
    fn close(&self, say: impl FnOnce()) -> bool {
        let mut g = self.gate();
        if *g {
            return false;
        }
        *g = true;
        if !self.peer_closed.load(Ordering::Acquire) {
            say();
        }
        drop(g);
        self.st().finished = true;
        self.cv.notify_all();
        true
    }

    /// The stream is about to carry a new connection: once this returns, nothing of this one is sent any more (a send in progress
    /// is waited for). Its threads stop at their next look.
    pub fn seal(&self) {
        self.peer_closed.store(true, Ordering::Release);
        *self.gate() = true;
        let mut st = self.st();
        st.peer_done = true;
        st.finished = true;
        drop(st);
        self.cv.notify_all();
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
            kinds::PREVIEW_END | STREAM_CLOSE => {
                self.peer_closed.store(true, Ordering::Release);
                st.peer_done = true;
                st.done_at.get_or_insert_with(Instant::now);
            }
            _ => {}
        }
        drop(st);
        self.cv.notify_all();
    }

    /// The local connection that carried the stream went: the connection ends without a word.
    pub fn gone(&self) {
        self.peer_closed.store(true, Ordering::Release);
        let mut st = self.st();
        st.peer_done = true;
        st.done_at.get_or_insert_with(Instant::now);
        drop(st);
        self.cv.notify_all();
    }

    /// Whether the window ended the connection (or the local connection went).
    pub fn peer_closed(&self) -> bool {
        self.peer_closed.load(Ordering::Acquire)
    }

    /// Whether the connection is over: nothing of it is sent any more.
    pub fn finished(&self) -> bool {
        *self.gate()
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

    /// Ends the connection once: with a reason, `PREVIEW_END` (the bytes both ways) then `STREAM_CLOSE`, written under the gate so
    /// nothing follows them; without one (or when the window ended it), quietly.
    fn end(&self, reason: Option<Reason>) {
        let closed = self.inbox.close(|| {
            if let Some(reason) = reason {
                let end = v1::PreviewEnd {
                    reason: reason as i32,
                    bytes_to_target: self.to_target.load(Ordering::Acquire),
                    bytes_from_target: self.from_target.load(Ordering::Acquire),
                };
                let _ = (self.send)(kinds::PREVIEW_END, end.encode_to_vec());
                let _ = (self.send)(STREAM_CLOSE, Vec::new());
            }
        });
        if closed || self.inbox.peer_closed.load(Ordering::Acquire) {
            let _ = self.tcp.shutdown(Shutdown::Both);
        }
    }

    fn is_ended(&self) -> bool {
        self.inbox.finished()
    }

    /// Counts `n` bytes against the forward's total and this connection's rate. `wait`: this side may wait for the next second
    /// instead of going over the rate (bytes it read); the window's bytes cannot wait, so going over ends the connection.
    fn spend(&self, n: u64, wait: bool) -> Spent {
        let b = self.budgets();
        if !self.lease.live.count(n, b.total) {
            return Spent::Over;
        }
        if b.rate == 0 {
            return Spent::Ok;
        }
        let rate = u64::from(b.rate);
        loop {
            let mut r = self.rate.lock().unwrap_or_else(|e| e.into_inner());
            if r.0.elapsed() >= Duration::from_secs(1) {
                *r = (Instant::now(), 0);
            }
            if r.1 + n <= rate {
                r.1 += n;
                return Spent::Ok;
            }
            if !wait || n > rate {
                return Spent::Over;
            }
            let left = Duration::from_secs(1).saturating_sub(r.0.elapsed());
            drop(r);
            std::thread::sleep(left.min(POLL));
            if self.is_ended() || self.outside().is_some() {
                return Spent::Ended;
            }
        }
    }
}

/// What [`Conn::spend`] found.
enum Spent {
    Ok,
    /// Over the rate or the forward's total.
    Over,
    /// The connection ended while it waited: nothing more is sent.
    Ended,
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
    inbox.close(|| {
        let end = v1::PreviewEnd {
            reason: Reason::Budget as i32,
            bytes_to_target: 0,
            bytes_from_target: 0,
        };
        let _ = send(kinds::PREVIEW_END, end.encode_to_vec());
        let _ = send(STREAM_CLOSE, Vec::new());
    });
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
        // The window broke the rules while the other thread is held in a write to a program that does not read: ended from here,
        // and the shutdown frees that write.
        {
            let st = c.inbox.st();
            if st.violation {
                drop(st);
                return c.end(Some(Reason::Protocol));
            }
            if st.done_at.is_some_and(|t| t.elapsed() >= DRAIN) {
                drop(st);
                return c.end(None);
            }
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
                match c.spend(n as u64, true) {
                    Spent::Ok => {}
                    Spent::Over => return c.end(Some(Reason::Budget)),
                    Spent::Ended => return c.end(c.outside().flatten()),
                }
                c.inbox.st().may_send -= n as u64;
                // Through the gate: once the connection is over, what was read is dropped, never sent after the end.
                let counted = || {
                    c.from_target.fetch_add(n as u64, Ordering::AcqRel);
                };
                if !c
                    .inbox
                    .send_open(&c.send, kinds::PREVIEW_DATA, buf[..n].to_vec(), counted)
                {
                    return c.end(None);
                }
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
            match c.spend(n, false) {
                Spent::Ok => {}
                Spent::Over => return c.end(Some(Reason::Budget)),
                Spent::Ended => return c.end(c.outside().flatten()),
            }
            if tcp.write_all(&bytes).is_err() {
                return c.end(Some(Reason::TargetGone));
            }
            c.to_target.fetch_add(n, Ordering::AcqRel);
            c.inbox.st().may_receive += n;
            let credit = v1::PreviewCredit {
                bytes: u32::try_from(n).unwrap_or(u32::MAX),
            };
            if !c.inbox.send_open(
                &c.send,
                kinds::PREVIEW_CREDIT,
                credit.encode_to_vec(),
                || {},
            ) {
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

    /// Review of d267e547: once a connection is over nothing of it is sent, and sealing a stream for a new connection waits for a
    /// send already under way, so no old frame can follow the new connection's answer.
    #[test]
    fn nothing_is_sent_after_the_end_and_a_seal_waits_for_a_send_under_way() {
        let i = Arc::new(Inbox::new(&small()));
        let sent = Arc::new(Mutex::new(Vec::<u8>::new()));
        // The send says when it has begun, so the seal below is sure to find it under way (a fixed sleep was not enough on a busy
        // macOS runner: the thread had not started yet, and the seal had nothing to wait for).
        let (begun, has_begun) = std::sync::mpsc::sync_channel::<()>(1);
        let slow: Send = {
            let sent = Arc::clone(&sent);
            Arc::new(move |kind, _| {
                let _ = begun.try_send(());
                std::thread::sleep(Duration::from_millis(300));
                sent.lock().unwrap().push(kind);
                Ok(())
            })
        };
        let (i2, s2) = (Arc::clone(&i), Arc::clone(&slow));
        let under_way =
            std::thread::spawn(move || i2.send_open(&s2, kinds::PREVIEW_DATA, vec![1], || {}));
        assert!(
            has_begun.recv_timeout(Duration::from_secs(30)).is_ok(),
            "the send began"
        );
        let began = Instant::now();
        i.seal();
        assert!(
            began.elapsed() >= Duration::from_millis(200),
            "the seal waited for the send under way"
        );
        assert_eq!(
            *sent.lock().unwrap(),
            vec![kinds::PREVIEW_DATA],
            "and that send was finished when it returned"
        );
        assert!(under_way.join().unwrap());
        assert!(
            !i.send_open(&slow, kinds::PREVIEW_DATA, vec![2], || {}),
            "nothing after the seal"
        );
        assert!(!i.close(|| panic!("no end is said twice")));

        let j = Inbox::new(&small());
        assert!(j.close(|| {}));
        assert!(
            !j.send_open(&slow, kinds::PREVIEW_CREDIT, vec![], || {}),
            "nothing after the end"
        );
        let k = Inbox::new(&small());
        k.push(STREAM_CLOSE, &[]);
        assert!(
            !k.send_open(&slow, kinds::PREVIEW_DATA, vec![3], || {}),
            "nothing after the window ended it"
        );
        assert!(k.close(|| panic!("nothing is said back to a window that ended it")));
    }

    #[test]
    fn the_end_from_the_window_and_a_finished_connection() {
        let i = Inbox::new(&small());
        i.push(STREAM_CLOSE, &[]);
        assert!(i.st().peer_done);
        assert!(i.close(|| {}));
        i.push(kinds::PREVIEW_DATA, b"late");
        assert!(i.st().buf.is_empty(), "frames after the end are dropped");
        assert!(i.finished());
    }
}
