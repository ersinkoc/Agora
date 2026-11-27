//! ADR-0079 W2 step 2: the node's use of its terminal holder (`agora hold`), so that agents outlive the node (Unix).
//!
//! The holder is the agent's parent and keeps its terminal; the node reads and writes the terminal itself, through the master the holder
//! handed over (`agora_platform::held_pty`), so no byte passes through the holder while the node is attached. What only a parent knows
//! (how the agent ended, a signal to its group) goes over the link ([`crate::holder_link`]). `docs/specs/holder-link.md` is the
//! specification.
//!
//! Everything here is behind a safe fallback: where there is no holder (not built, it cannot start, the link fails), [`HolderHub`]
//! says so and the supervisor runs the agent itself exactly as before. A holder is started by the first agent that needs one, never at
//! the node's start, so a node that never starts an agent never leaves an idle holder behind.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use agora_platform::{ExitStatus, HeldControl, HeldSignal, ProcessWatch, TerminalHandle};

use crate::holder_link::{
    socket_path, AgentInfo, AgentState, Client, Held, LinkError, Refusal, LINK_VERSION_PIPES,
    MAX_FRAME,
};
pub use crate::holder_link::{HeldPipe, StartPipeRequest, StartRequest};

/// How long a start waits for a holder it started to listen. Together with [`BUSY_WAIT`] and [`ENDING_WAIT`] (a start whose holder
/// closed the link waits that long for it to end, #951) this bounds what a start can hold the supervisor for, once per [`BACKOFF`]
/// when the holder cannot be had.
const START_WAIT: Duration = Duration::from_secs(3);
/// How long a connection retries `Busy` (the holder has not noticed that the node before this one is gone).
const BUSY_WAIT: Duration = Duration::from_secs(1);
/// How long ending an agent through the holder waits for it to be collected.
const END_WAIT: Duration = Duration::from_secs(3);
/// How long a start that found the holder closing waits for its process to end (it ends right after it answered the release of its last
/// agent; a few milliseconds in practice).
const ENDING_WAIT: Duration = Duration::from_secs(1);
/// After a holder could not be started or reached, new agents run in the node for this long before it is tried again.
const BACKOFF: Duration = Duration::from_secs(30);

/// Where the node's holder lives and how to start it.
#[derive(Debug, Clone)]
pub struct HolderSetup {
    /// The Agora home (the holder's socket is `<home>/run/hold.sock`).
    pub home: PathBuf,
    /// The `agora` binary: `agora hold --home <home>` is the holder.
    pub exe: PathBuf,
}

/// One connection to the holder. Every call goes through one lock, and a failed link marks it broken for good: [`Self::gone`].
pub struct HolderSession {
    client: Mutex<Client>,
    watch: ProcessWatch,
    broken: AtomicBool,
    /// Test hook ([`HolderHub::lose_next_start_reply`]): the next start is made but its answer is lost.
    lose_reply: Arc<AtomicBool>,
    /// The holder's process, as the ledger records it (ADR-0079 decision 9).
    pub pid: u32,
    pub start: u64,
    /// ADR-0093: the holder keeps structured agents on pipes (it spoke link version 2 or later). An older holder does not, and is
    /// never sent what it cannot read.
    pub pipes: bool,
}

fn to_io(e: LinkError) -> io::Error {
    match e {
        LinkError::Io(e) => e,
        other => io::Error::other(other.to_string()),
    }
}

impl HolderSession {
    /// Whether the holder is gone (its process ended or the link failed): nothing more can be asked of it (decision 8).
    pub fn gone(&self) -> bool {
        self.broken.load(Ordering::Acquire) || self.watch.ended()
    }

    fn client(&self) -> MutexGuard<'_, Client> {
        self.client.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn call<R>(&self, f: impl FnOnce(&mut Client) -> Result<R, LinkError>) -> Result<R, LinkError> {
        if self.gone() {
            return Err(LinkError::Closed);
        }
        let r = f(&mut self.client());
        if matches!(
            r,
            Err(LinkError::Io(_) | LinkError::Closed | LinkError::Malformed(_))
        ) {
            self.broken.store(true, Ordering::Release);
        }
        r
    }

    /// Starts an agent in the holder: its pid and its terminal, ready to read (the master is close-on-exec).
    pub fn start(&self, request: &StartRequest) -> Result<(u32, TerminalHandle), LinkError> {
        let started = self.call(|c| c.start(request));
        if started.is_ok() && self.lose_reply.swap(false, Ordering::AcqRel) {
            // The agent runs in the holder; the node never learns it (what a link that fails after the start looks like).
            return Err(LinkError::Io(io::Error::from(io::ErrorKind::TimedOut)));
        }
        started
    }

    /// ADR-0093: starts a program on pipes in the holder; its pid.
    pub fn start_pipe(&self, request: &StartPipeRequest) -> Result<u32, LinkError> {
        if !self.pipes {
            return Err(LinkError::Refused(Refusal::BadRequest, 0));
        }
        self.call(|c| c.start_pipe(request))
    }

    fn pipe_read(&self, agent: &str) -> Result<crate::holder_link::PipeData, LinkError> {
        self.call(|c| c.pipe_read(agent))
    }

    fn pipe_write(&self, agent: &str, line: &[u8]) -> Result<(), LinkError> {
        self.call(|c| c.pipe_write(agent, line))
    }

    /// ADR-0093: the two ends of a pipe agent the holder runs, as the adapter host reads and writes them. `fresh` and `dropped` are what
    /// `AttachPipes` said no node had: the output starts with those lines.
    pub fn pipe_io(
        self: &Arc<Self>,
        agent: &str,
        fresh: Vec<Vec<u8>>,
        dropped: u64,
        pid: u32,
    ) -> agora_adapters::host::HeldIo {
        let stop = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicU64::new(dropped));
        let ready = Arc::new(AtomicBool::new(false));
        let exit = Arc::new(std::sync::Mutex::new(None));
        let mut first = Vec::new();
        for l in fresh {
            first.extend_from_slice(&l);
            first.push(b'\n');
        }
        agora_adapters::host::HeldIo {
            pid,
            input: Box::new(PipeWriter {
                session: Arc::clone(self),
                agent: agent.to_string(),
                line: Vec::new(),
            }),
            output: Box::new(PipeReader {
                session: Arc::clone(self),
                agent: agent.to_string(),
                buf: first,
                pos: 0,
                stop: Arc::clone(&stop),
                dropped: Arc::clone(&dropped),
                ready: Arc::clone(&ready),
                exit: Arc::clone(&exit),
                ended_reads: 0,
                wait: PIPE_POLL_MIN,
                exited_idle: None,
                failures: 0,
            }),
            control: self.control_of(agent),
            stop,
            dropped,
            ready,
            exit,
        }
    }

    /// Ends `agent` for good: stops its group, waits until the holder reports it ended, and releases it (so the holder forgets it,
    /// and ends when that was its last). `Ok` also when the holder does not know it. `Err` when that could not be settled (the link
    /// failed, or it did not end in time): nothing is known of it then.
    pub fn end_agent(&self, agent: &str) -> io::Result<()> {
        let end = Instant::now() + END_WAIT;
        let mut stopped = false;
        loop {
            let list = self.call(|c| c.list()).map_err(to_io)?;
            match list.into_iter().find(|a| a.agent == agent) {
                None => return Ok(()),
                Some(AgentInfo {
                    state: AgentState::Exited { .. },
                    ..
                }) => return self.release(agent).map_err(to_io),
                Some(_) => {}
            }
            if !stopped {
                self.stop(agent, true).map_err(to_io)?;
                stopped = true;
            }
            if Instant::now() >= end {
                return Err(io::Error::from(io::ErrorKind::TimedOut));
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    /// Whether the holder is gone after a start failed with `e` (its process, not only the link, so an agent it started is not running on
    /// its account): its process has ended, or the link failed with anything but a time-out and its process ends within
    /// [`ENDING_WAIT`]. A holder ends right after the node releases its last agent (an agent that crashed, collected before its restart),
    /// so the restart's start can meet a holder that is closing but whose process has not ended yet (#951): that holder started nothing,
    /// and waiting for its end lets the restart start a new one instead of failing. A reply that did not come in time is not waited for
    /// (that holder is alive and may have started the agent).
    pub fn ended_after(&self, e: &LinkError) -> bool {
        // Any I/O error but a time-out may be a holder that is ending: besides end of stream, a broken pipe or a reset, macOS can
        // answer a send to a Unix socket whose peer is closing with `EPROTOTYPE`, which has no `ErrorKind` of its own. Waiting is still
        // bounded, and the start goes on only once the process has really ended.
        let closed = match e {
            LinkError::Closed => true,
            LinkError::Io(e) => !matches!(
                e.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ),
            _ => false,
        };
        self.watch.ended() || (closed && self.watch.wait(ENDING_WAIT))
    }

    /// Stops an agent's process group through the holder (`SIGKILL` when `force`).
    pub fn stop(&self, agent: &str, force: bool) -> Result<(), LinkError> {
        self.call(|c| c.stop(agent, force))
    }

    /// Tells the holder an ended agent's status and output were taken (it forgets it, and ends when that was its last agent).
    pub fn release(&self, agent: &str) -> Result<(), LinkError> {
        self.call(|c| c.release(agent))
    }

    /// The control a held agent's terminal asks the holder through.
    pub fn control_of(self: &Arc<Self>, agent: &str) -> Arc<dyn HeldControl> {
        Arc::new(AgentLink {
            session: Arc::clone(self),
            agent: agent.to_string(),
        })
    }
}

/// The shortest and the longest wait between two reads of an idle pipe agent's output. Every line waits at most the longest.
const PIPE_POLL_MIN: Duration = Duration::from_millis(2);
const PIPE_POLL_MAX: Duration = Duration::from_millis(40);
/// An agent that ended and whose output stays open (something it started holds it) is over after this long without a line.
const PIPE_EXIT_GRACE: Duration = Duration::from_millis(500);
/// Reads after the output ended while the holder still says the agent runs, to hear how it ended (#902).
const PIPE_EOF_READS: u32 = 5;

/// The output of a pipe agent in the holder, as a stream of lines for the adapter host's reader: what the holder kept first, then what
/// it writes. A holder or a link that cannot be asked is the end of the stream (the adapter then counts as ended).
struct PipeReader {
    session: Arc<HolderSession>,
    agent: String,
    buf: Vec<u8>,
    pos: usize,
    stop: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
    /// Set once what the holder kept has been handed to the reader.
    ready: Arc<AtomicBool>,
    /// #902: how the agent ended, as the holder says with its output; published before the output ends, read by the node without a
    /// question of its own (`Adapter::exit_status`).
    exit: Arc<std::sync::Mutex<Option<agora_platform::ExitStatus>>>,
    /// Reads since the output ended while the holder still said the agent ran (it closes its output a moment before it exits).
    ended_reads: u32,
    wait: Duration,
    exited_idle: Option<Instant>,
    /// Reads in a row the holder refused for a passing reason.
    failures: u32,
}

/// How many times in a row a refusal that may pass (the holder could not do it just now) is tried again before the agent counts as gone,
/// and the wait between them: a hiccup must not end an agent and have it restarted.
const PIPE_RETRIES: u32 = 20;
const PIPE_RETRY_WAIT: Duration = Duration::from_millis(50);

/// Whether a failed read of a pipe agent's output is its end. A link that failed, a holder that no longer knows the agent or does not
/// serve this node are final (the session is broken for good, or there is nothing to read); any other refusal is retried.
fn read_error_ends(e: &LinkError, failures: u32) -> bool {
    match e {
        LinkError::Refused(Refusal::Failed | Refusal::Busy, _) => failures > PIPE_RETRIES,
        _ => true,
    }
}

impl Read for PipeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            if self.pos < self.buf.len() {
                let n = out.len().min(self.buf.len() - self.pos);
                out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
                self.pos += n;
                if self.pos == self.buf.len() {
                    self.buf.clear();
                    self.pos = 0;
                }
                return Ok(n);
            }
            self.ready.store(true, Ordering::Release);
            if self.stop.load(Ordering::Acquire) {
                return Ok(0);
            }
            let data = match self.session.pipe_read(&self.agent) {
                Ok(d) => {
                    self.failures = 0;
                    d
                }
                Err(e) => {
                    self.failures += 1;
                    if read_error_ends(&e, self.failures) {
                        return Ok(0);
                    }
                    std::thread::sleep(PIPE_RETRY_WAIT);
                    continue;
                }
            };
            if let AgentState::Exited { code, signal } = data.info.state {
                if let Ok(mut exit) = self.exit.lock() {
                    *exit = Some(agora_platform::ExitStatus { code, signal });
                }
            }
            if data.dropped > 0 {
                self.dropped.fetch_add(data.dropped, Ordering::AcqRel);
            }
            if !data.fresh.is_empty() {
                for l in &data.fresh {
                    self.buf.extend_from_slice(l);
                    self.buf.push(b'\n');
                }
                self.wait = PIPE_POLL_MIN;
                self.exited_idle = None;
                continue;
            }
            // The output ended: over once the holder says how the agent ended, or after a few more reads (off any lock of the node's;
            // at most `PIPE_EOF_READS` times `PIPE_POLL_MAX`) when it still says it runs; its status is then not known.
            if data.eof {
                if matches!(data.info.state, AgentState::Exited { .. })
                    || self.ended_reads >= PIPE_EOF_READS
                {
                    return Ok(0);
                }
                self.ended_reads += 1;
                std::thread::sleep(PIPE_POLL_MAX);
                continue;
            }
            if matches!(data.info.state, AgentState::Exited { .. })
                && self.exited_idle.get_or_insert_with(Instant::now).elapsed() >= PIPE_EXIT_GRACE
            {
                return Ok(0);
            }
            std::thread::sleep(self.wait);
            self.wait = (self.wait * 2).min(PIPE_POLL_MAX);
        }
    }
}

/// The input of a pipe agent in the holder: each complete line is one frame. A line too large for a frame fails that write only, never
/// the link.
struct PipeWriter {
    session: Arc<HolderSession>,
    agent: String,
    line: Vec<u8>,
}

impl Write for PipeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.line.extend_from_slice(buf);
        while let Some(i) = self.line.iter().position(|b| *b == b'\n') {
            let rest = self.line.split_off(i + 1);
            let mut line = std::mem::replace(&mut self.line, rest);
            line.pop();
            if line.len() >= MAX_FRAME - 4096 {
                self.line.clear();
                return Err(io::Error::from(io::ErrorKind::InvalidInput));
            }
            if self.session.pipe_write(&self.agent, &line).is_err() {
                return Err(io::Error::from(io::ErrorKind::BrokenPipe));
            }
        }
        if self.line.len() >= MAX_FRAME - 4096 {
            self.line.clear();
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// [`HeldControl`] of one agent: signals and its end, asked of the holder.
struct AgentLink {
    session: Arc<HolderSession>,
    agent: String,
}

impl HeldControl for AgentLink {
    fn signal(&self, signal: HeldSignal) -> io::Result<()> {
        self.session
            .call(|c| c.signal(&self.agent, signal))
            .map_err(to_io)
    }

    fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        self.session
            .call(|c| c.resize(&self.agent, cols, rows))
            .map_err(to_io)
    }

    fn exit_status(&self) -> io::Result<Option<ExitStatus>> {
        let list = self.session.call(|c| c.list()).map_err(to_io)?;
        match list.into_iter().find(|a| a.agent == self.agent) {
            Some(AgentInfo {
                state: AgentState::Exited { code, signal },
                ..
            }) => {
                // Taken: the holder forgets it, and ends when that was its last agent.
                let _ = self.session.release(&self.agent);
                Ok(Some(ExitStatus { code, signal }))
            }
            Some(_) => Ok(None),
            // The holder does not know it any more: its end cannot be known.
            None => Err(io::Error::from(io::ErrorKind::NotFound)),
        }
    }
}

/// Why a connection did not come about.
enum Why {
    /// Nothing listens: no holder runs.
    NoHolder,
    Other,
}

/// What a new connection handed over: terminals and (ADR-0093) pipe agents.
struct Opened {
    session: Arc<HolderSession>,
    held: Vec<Held>,
    pipes: Vec<HeldPipe>,
}

fn open(setup: &HolderSetup, lose_reply: &Arc<AtomicBool>) -> Result<Opened, Why> {
    let socket = socket_path(&setup.home);
    let end = Instant::now() + BUSY_WAIT;
    // `Busy` is "try again" (the holder has not noticed that the node before this one is gone yet), never "a second node exists".
    let mut client = loop {
        match Client::connect(&socket) {
            Ok(c) => break c,
            Err(LinkError::Refused(Refusal::Busy, _)) if Instant::now() < end => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(LinkError::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                return Err(Why::NoHolder)
            }
            Err(_) => return Err(Why::Other),
        }
    };
    let (pid, start) = (client.holder_pid, client.holder_start);
    let watch = ProcessWatch::open(pid, start).map_err(|_| Why::Other)?;
    // Everything but `List` waits for `Attach`; a new holder has nothing to hand over.
    let held = client.attach().map_err(|_| Why::Other)?;
    // An older holder is never sent what it cannot read (it would close the link): it has no pipe agents either.
    let pipes_ok = client.version >= LINK_VERSION_PIPES;
    // What `agora doctor` and `agora status` cannot ask (the holder admits only the node): the version agreed, as a note.
    crate::holder_note::write(&setup.home, client.version, pid, start);
    let pipes = if pipes_ok {
        client.attach_pipes().map_err(|_| Why::Other)?
    } else {
        Vec::new()
    };
    let session = Arc::new(HolderSession {
        client: Mutex::new(client),
        watch,
        broken: AtomicBool::new(false),
        lose_reply: Arc::clone(lose_reply),
        pid,
        start,
        pipes: pipes_ok,
    });
    Ok(Opened {
        session,
        held,
        pipes,
    })
}

/// Starts `agora hold` detached, in a session of its own: never bound to its starter (a holder that died with the node would defeat
/// it). `Err` when it cannot be started that way (`setsid` refused; on Windows a job that refuses the breakaway: never a holder inside
/// the supervisor's kill-on-close job, ADR-0079 decision 2), which is "no holder here".
fn start_holder(setup: &HolderSetup) -> io::Result<u32> {
    let run = setup.home.join("run");
    agora_platform::ensure_private_dir(&run)?;
    let mut cmd = std::process::Command::new(&setup.exe);
    cmd.arg("hold").arg("--home").arg(&setup.home);
    Ok(agora_platform::spawn_detached_unbound(&mut cmd, &run.join("hold.log"))?.pid)
}

#[derive(Default)]
struct HubState {
    session: Option<Arc<HolderSession>>,
    /// No new attempt before this (a holder that could not be started or reached).
    unavailable_until: Option<Instant>,
}

/// The node's holder: finds the running one, or starts one when an agent needs it, and says "none" when it cannot.
pub struct HolderHub {
    setup: HolderSetup,
    state: Mutex<HubState>,
    /// The session in use, readable without waiting for a start in progress (the supervision tick asks which holder agents are in).
    current: Mutex<Option<Arc<HolderSession>>>,
    lose_reply: Arc<AtomicBool>,
}

impl HolderHub {
    pub fn new(setup: HolderSetup) -> Self {
        Self {
            setup,
            state: Mutex::new(HubState::default()),
            current: Mutex::new(None),
            lose_reply: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The session is not to be used any more (an agent it handed over could not be settled): the next start tries again after
    /// [`BACKOFF`], ending whatever the holder still holds that this node does not track.
    pub fn retire_session(&self) {
        let mut st = self.st();
        st.session = None;
        st.unavailable_until = Some(Instant::now() + BACKOFF);
        self.set_current(None);
    }

    /// Test hook: the next agent the holder starts for this node is started but its answer is lost.
    #[doc(hidden)]
    pub fn lose_next_start_reply(&self) {
        self.lose_reply.store(true, Ordering::Release);
    }

    fn set_current(&self, s: Option<Arc<HolderSession>>) {
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = s;
    }

    /// Ends every agent a new attach handed over that this node does not track (ending a running one, releasing an ended one), so none
    /// is left attached and unread: the holder stopped reading it at the attach, and the node drops the master. `false` when one could
    /// not be settled; the session is then not to be used.
    pub fn end_unadopted(session: &HolderSession, held: Vec<Held>) -> bool {
        Self::end_unadopted_all(session, held, Vec::new())
    }

    /// [`Self::end_unadopted`] with the pipe agents (ADR-0093) a new attach handed over and this node does not track: ended too, since
    /// an agent nobody reads is an agent nobody answers.
    pub fn end_unadopted_all(
        session: &HolderSession,
        held: Vec<Held>,
        pipes: Vec<HeldPipe>,
    ) -> bool {
        let mut all = true;
        for p in pipes {
            let id = p.info.agent.clone();
            let settled = match p.info.state {
                AgentState::Running => session.end_agent(&id),
                AgentState::Exited { .. } => session.release(&id).map_err(to_io),
            };
            all &= settled.is_ok();
        }
        for h in held {
            let id = h.info.info.agent.clone();
            let settled = match h.info.info.state {
                AgentState::Running => session.end_agent(&id),
                AgentState::Exited { .. } => session.release(&id).map_err(to_io),
            };
            all &= settled.is_ok();
        }
        all
    }

    fn st(&self) -> MutexGuard<'_, HubState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// At the node's start: attaches to the holder that is already running (what a node before this one left), with the terminals and
    /// output it holds. `None` when no holder runs or it cannot be used (then agents run in the node).
    pub fn attach_existing(&self) -> Option<(Arc<HolderSession>, Vec<Held>, Vec<HeldPipe>)> {
        if !agora_platform::HOLDER_BUILT {
            return None;
        }
        let mut st = self.st();
        match open(&self.setup, &self.lose_reply) {
            Ok(Opened {
                session,
                held,
                pipes,
            }) => {
                st.session = Some(Arc::clone(&session));
                self.set_current(Some(Arc::clone(&session)));
                Some((session, held, pipes))
            }
            Err(_) => None,
        }
    }

    /// A live session for a new agent: the one the node has, or a new one, starting the holder when none runs. `None` when there is no
    /// holder to be had, which makes the caller run the agent itself.
    pub fn session(&self) -> Option<Arc<HolderSession>> {
        if !agora_platform::HOLDER_BUILT {
            return None;
        }
        let mut st = self.st();
        if let Some(s) = st.session.as_ref().filter(|s| !s.gone()) {
            return Some(Arc::clone(s));
        }
        st.session = None;
        self.set_current(None);
        if st.unavailable_until.is_some_and(|t| Instant::now() < t) {
            return None;
        }
        let session = match open(&self.setup, &self.lose_reply) {
            Ok(Opened {
                session: s,
                held,
                pipes,
            }) => {
                // A holder that still holds agents this node does not track (its own link broke while the holder lived: the node
                // already counts them ended): they are ended, not left attached and unread, and the session is used only if all were.
                if Self::end_unadopted_all(&s, held, pipes) {
                    Some(s)
                } else {
                    None
                }
            }
            Err(Why::Other) => None,
            Err(Why::NoHolder) => self.start_and_open(),
        };
        match session {
            Some(s) => {
                st.unavailable_until = None;
                st.session = Some(Arc::clone(&s));
                self.set_current(Some(Arc::clone(&s)));
                Some(s)
            }
            None => {
                st.unavailable_until = Some(Instant::now() + BACKOFF);
                None
            }
        }
    }

    fn start_and_open(&self) -> Option<Arc<HolderSession>> {
        let pid = start_holder(&self.setup).ok()?;
        let end = Instant::now() + START_WAIT;
        loop {
            match open(&self.setup, &self.lose_reply) {
                Ok(Opened {
                    session: s,
                    held,
                    pipes,
                }) if held.is_empty() && pipes.is_empty() => return Some(s),
                Err(Why::NoHolder) if Instant::now() < end => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                _ => {
                    // It started but cannot be used (it does not admit this node, a different version): never left idle.
                    let _ = agora_platform::terminate_process(pid);
                    return None;
                }
            }
        }
    }

    /// The holder's process, as the ledger records it, while a live session exists.
    pub fn identity(&self) -> Option<(u32, u64)> {
        self.current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .filter(|s| !s.gone())
            .map(|s| (s.pid, s.start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_final_error_ends_a_pipe_agent_and_a_passing_refusal_is_tried_again_a_while() {
        let io = LinkError::Io(io::Error::from(io::ErrorKind::TimedOut));
        assert!(read_error_ends(&io, 1));
        assert!(read_error_ends(&LinkError::Closed, 1));
        assert!(read_error_ends(&LinkError::Malformed("x"), 1));
        assert!(read_error_ends(
            &LinkError::Refused(Refusal::UnknownAgent, 1),
            1
        ));
        let hiccup = LinkError::Refused(Refusal::Failed, 1);
        assert!(!read_error_ends(&hiccup, 1));
        assert!(!read_error_ends(&hiccup, PIPE_RETRIES));
        assert!(read_error_ends(&hiccup, PIPE_RETRIES + 1));
    }
}
