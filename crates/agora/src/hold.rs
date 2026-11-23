//! `agora hold`: the terminal holder (ADR-0079), the process that owns the agents' terminals so they outlive their node.
//!
//! The holder never decides anything. It starts, resizes, signals and stops an agent only when its node asks, keeps the terminals,
//! reads them into a bounded buffer while no node is attached, and keeps the exit status and unread output of agents that ended with
//! no node there until a node takes them. It serves one node at a time, and only the process that holds this home's node lock, by
//! pid and start time (ADR-0062 item 8). The link is `agora_node::holder_link`; `docs/specs/holder-link.md` describes it.
//!
//! One thread serves the node and starts every agent (a thread of its own per pipe agent only reads its output and writes its input,
//! ADR-0093; none of them decides anything), so the process that starts each agent lives as long as the holder (on Linux the agent's first process
//! gets `SIGHUP` when it ends, which the kernel ties to the thread that forked). The node starts it and starts its agents through it
//! (W2 step 2, `agora_node::holder`). It writes `run/hold.pid` (its pid, start time and how many agents run), which `agora down` and
//! `agora status` read, and on `SIGTERM` (`agora down --agents`) it ends its agents and itself.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use agora_i18n::key;
use agora_node::holder_link::{
    agent_id_fits, read_frame, socket_path, write_frame, AgentInfo, AgentState, PipeData, Refusal,
    Reply, Request, StartPipeRequest, StartRequest, TerminalInfo, LINK_VERSION, LINK_VERSION_PIPES,
    MAX_FRAME, MAX_HELD_AGENTS, WRITE_SLICE,
};
use agora_platform::{
    ExitStatus, HeldBuffer, HeldProcess, HeldSignal, HeldTerminal, LineWriter, LocalListener,
    LocalStream, Peer, PipeLines, ProcessWatch, PtyConfig, QueueError, TerminalReader,
};

use crate::frontdoor::{read_lock, Role};
use crate::{home, Opts, Say};

/// How often the holder looks for a node's end, an ended agent and a new connection.
const TICK: Duration = Duration::from_millis(50);

/// How long a peer that passed the pid check may take to begin its hello. One that begins a frame and stalls holds the holder for up
/// to [`FRAME_TIMEOUT`] (5 s); only the process that holds the node lock can be that peer.
const HELLO_WAIT: Duration = Duration::from_secs(1);

/// Connections taken in one pass. The node is served first in every pass, so a flood of connections (each is checked and closed at
/// once) slows the node by at most this many checks and can never starve it, a reap or a watch.
const MAX_ACCEPTS: usize = 16;

/// `agora hold`.
pub(crate) fn hold(say: &Say, opts: &Opts) -> ExitCode {
    // First of all: what the starter handed this process (its pipes, its logs) is nobody's business here, and an agent must never get it.
    agora_platform::close_inherited_descriptors();
    let home = opts.home.clone().unwrap_or_else(home);
    // Its own session: not ended with the terminal or the process group that started it (decision 2). A process that leads a group
    // cannot, and a holder left in its starter's session would end with it: it refuses to start (the starter must detach it).
    if let Err(e) = agora_platform::start_new_session() {
        say.err(key!("hold-failed"), &[("message", e.to_string().into())]);
        return ExitCode::from(1);
    }
    // `agora down --agents` asks it to end by `SIGTERM`: its agents end first (a holder killed outright would leave them the kernel's
    // hangup only). Not being able to catch it is not a reason not to hold anything.
    let _ = agora_platform::catch_stop_requests();
    let lock = Role::Node.lock(&home);
    let admit: Admit = Box::new(move |peer| admit_by_lock(&lock, peer));
    match Holder::open(&home, admit).map(Holder::run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            say.err(key!("hold-failed"), &[("message", e.to_string().into())]);
            ExitCode::from(1)
        }
    }
}

/// Whether `peer` is the node holding the role lock now. Returns the peer's start time.
///
/// The lock names the process that holds the role: a supervisor, which runs the node as its child (ADR-0062 item 8), or the node itself
/// when it holds the lock. So a peer is admitted when it is that process (its pid and its start time, read now, are the lock's: a pid
/// another process got after the node ended is not admitted) or its direct child, an `agora` process, while the lock holder is alive
/// (the supervisor's node).
pub(crate) fn admit_by_lock(lock: &Path, peer: &Peer) -> Option<u64> {
    if !peer.same_user {
        return None;
    }
    let l = read_lock(lock)?;
    let start = agora_platform::process_start_time(peer.pid)?;
    if l.pid == peer.pid {
        return (l.start == start).then_some(start);
    }
    // The supervisor's one child is the node, an `agora` process. The name is the process's own to set, so it is not what makes this
    // safe: a supervisor starts nothing but its node, and an agent (a confined one too) is the node's or the holder's child, never a
    // direct child of the lock holder. The name only keeps a lock held by something else (a test, a node that holds it itself) from
    // admitting whatever it started.
    let child = l.alive()
        && agora_platform::process_children(l.pid).is_ok_and(|c| c.contains(&peer.pid))
        && agora_platform::process_name(peer.pid)
            .is_ok_and(|n| n.to_lowercase().starts_with("agora"));
    child.then_some(start)
}

/// ADR-0079 W2 step 2: where this node's holder lives and how to start it, or `None` when it must not use one: no holder on this
/// system, or this process is not the one the role lock admits (its pid, or a child of the supervisor that holds the lock), in which
/// case a holder it started would refuse it and idle. The node then runs its agents itself, as before.
pub(crate) fn setup_for_node(home: &Path) -> Option<agora_node::holder::HolderSetup> {
    if !agora_platform::HOLDER_BUILT {
        return None;
    }
    let me = Peer {
        pid: std::process::id(),
        same_user: true,
    };
    admit_by_lock(&Role::Node.lock(home), &me)?;
    Some(agora_node::holder::HolderSetup {
        home: home.to_path_buf(),
        exe: std::env::current_exe().ok()?,
    })
}

/// `run/hold.pid`: `pid start agents`, so that `agora down` and `agora status` can find the holder and say how many agents it keeps.
pub(crate) fn info_path(home: &Path) -> PathBuf {
    home.join("run").join("hold.pid")
}

/// The running holder of `home` and how many agents it keeps running; `None` when there is none (the file is stale, or missing).
pub(crate) fn read_info(home: &Path) -> Option<(u32, usize)> {
    let text = std::fs::read_to_string(info_path(home)).ok()?;
    let mut w = text.split_whitespace();
    let (pid, start, agents): (u32, u64, usize) = (
        w.next()?.parse().ok()?,
        w.next()?.parse().ok()?,
        w.next()?.parse().ok()?,
    );
    (agora_platform::process_start_time(pid) == Some(start)).then_some((pid, agents))
}

/// Decides, from the kernel's record of who connected, whether a peer may use the holder: the verified start time, or `None`.
pub(crate) type Admit = Box<dyn Fn(&Peer) -> Option<u64>>;

struct Agent {
    id: String,
    term: HeldTerminal,
    /// Reading the terminal into `held`: only while no node is attached.
    reader: Option<TerminalReader>,
    /// Output read and not yet taken (a running agent's is taken at attach; an ended agent's stays until released).
    held: HeldBuffer,
    exit: Option<ExitStatus>,
    /// The terminal ended: every process closed its side, and nothing more can be read.
    eof: bool,
    /// When the node asked to end it (`Stop`): an ending agent that is still not reaped a moment later has nobody reading its terminal.
    ending: Option<Instant>,
}

/// How long an ending agent gets to leave by itself before the holder reads its terminal for it.
const ENDING_GRACE: Duration = Duration::from_millis(200);

impl Agent {
    /// Whether the holder must read this agent's terminal so that it can exit: it is being ended, has not been reaped for
    /// [`ENDING_GRACE`] (a node that reads its terminal would have let it go), no reader of ours runs, and no live node may read it
    /// (`node_may_read`: a node whose link dropped still holds its copy). macOS keeps a process that exits with unread terminal
    /// output in the exiting state until that output is read.
    fn must_drain_to_end(&self, now: Instant, node_may_read: bool) -> bool {
        !node_may_read
            && self.exit.is_none()
            && !self.eof
            && self.reader.is_none()
            && self
                .ending
                .is_some_and(|t| now.saturating_duration_since(t) >= ENDING_GRACE)
    }

    /// Reads what waits in the terminal into the buffer, when [`Self::must_drain_to_end`]; `true` when it did.
    fn drain_to_end(&mut self, now: Instant, node_may_read: bool) -> bool {
        if !self.must_drain_to_end(now, node_may_read) {
            return false;
        }
        self.eof = self.term.drain(&mut self.held).unwrap_or(true);
        true
    }

    fn state(&self) -> AgentState {
        match self.exit {
            None => AgentState::Running,
            Some(s) => AgentState::Exited {
                code: s.code,
                signal: s.signal,
            },
        }
    }

    fn info(&self) -> AgentInfo {
        AgentInfo {
            agent: self.id.clone(),
            pid: self.term.pid(),
            state: self.state(),
        }
    }

    /// Ends the read (waiting for it, so no byte is in flight) and keeps what it read.
    fn end_read(&mut self) {
        if let Some(r) = self.reader.take() {
            let c = r.end();
            self.eof |= c.ended;
            self.held.append(c.buffer);
        }
    }
}

/// ADR-0093: a structured agent on a pair of pipes. The holder never reads what it writes beyond cutting it into lines (its output is
/// read by a thread of its own into a bounded buffer, whether or not a node is there), and never parses a line: they are ACP's or an
/// adapter's own JSON-RPC, the node's to understand.
struct PipeAgent {
    id: String,
    proc: HeldProcess,
    lines: PipeLines,
    /// `None` once its input could not be taken or closed.
    input: Option<LineWriter>,
    exit: Option<ExitStatus>,
}

impl PipeAgent {
    fn state(&self) -> AgentState {
        match self.exit {
            None => AgentState::Running,
            Some(s) => AgentState::Exited {
                code: s.code,
                signal: s.signal,
            },
        }
    }

    fn info(&self) -> AgentInfo {
        AgentInfo {
            agent: self.id.clone(),
            pid: self.proc.pid(),
            state: self.state(),
        }
    }
}

struct Node {
    stream: LocalStream,
    /// The link version this node speaks (ADR-0093: version 2 may use pipe agents; a node of version 1 is never sent them).
    version: u16,
    watch: ProcessWatch,
    attached: bool,
}

pub(crate) struct Holder {
    listener: LocalListener,
    admit: Admit,
    agents: Vec<Agent>,
    /// ADR-0093: the structured agents.
    pipes: Vec<PipeAgent>,
    node: Option<Node>,
    /// The node whose link dropped while its process runs: it still holds its copies of the terminals and reads them itself, so the
    /// holder reads none until that process has ended (a read of a terminal two processes read blocks, and ending it would never
    /// return). `docs/specs/holder-link.md`.
    departed: Option<ProcessWatch>,
    /// An agent was ever started: a holder that never had one waits for its first.
    had_agents: bool,
    pid: u32,
    start: u64,
    /// The home, for `run/hold.pid`, and the count it last said.
    home: PathBuf,
    published: Option<usize>,
    /// The most agents it keeps (#983): [`MAX_HELD_AGENTS`], as many as one `List` reply can name; lower only in tests.
    capacity: usize,
}

impl Holder {
    /// Makes the home's `run` folder (owner-only) and listens on its socket.
    pub(crate) fn open(home: &Path, admit: Admit) -> std::io::Result<Self> {
        let socket: PathBuf = socket_path(home);
        if let Some(dir) = socket.parent() {
            agora_platform::ensure_private_dir(dir)?;
        }
        let listener = agora_platform::local_listen(&socket)?;
        listener.set_nonblocking(true)?;
        let pid = std::process::id();
        let start = agora_platform::process_start_time(pid).unwrap_or(0);
        Ok(Self {
            listener,
            admit,
            agents: Vec::new(),
            pipes: Vec::new(),
            node: None,
            departed: None,
            had_agents: false,
            pid,
            start,
            home: home.to_path_buf(),
            published: None,
            capacity: MAX_HELD_AGENTS,
        })
    }

    /// Serves until nothing is left to keep: a holder that held agents and has none left, after a node took the last.
    pub(crate) fn run(mut self) {
        while self.tick() {}
        let _ = std::fs::remove_file(info_path(&self.home));
    }

    /// Says (in `run/hold.pid`) how many agents run, when that changed. A failed write is only a stale count.
    fn publish(&mut self) {
        let running = self.agents.iter().filter(|a| a.exit.is_none()).count()
            + self.pipes.iter().filter(|a| a.exit.is_none()).count();
        if self.published != Some(running) {
            let text = format!("{} {} {running}\n", self.pid, self.start);
            if agora_platform::write_private(&info_path(&self.home), text.as_bytes()).is_ok() {
                self.published = Some(running);
            }
        }
    }

    /// `SIGTERM` (`agora down --agents`): every agent's group is asked to end, and killed when it has not within a few seconds.
    fn end_agents(&mut self) {
        for a in &self.agents {
            let _ = a.term.signal(HeldSignal::Terminate);
        }
        for a in &self.pipes {
            let _ = a.proc.signal(HeldSignal::Terminate);
        }
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end {
            self.reap();
            if self.agents.iter().all(|a| a.exit.is_some())
                && self.pipes.iter().all(|a| a.exit.is_some())
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        for a in &self.agents {
            let _ = a.term.signal(HeldSignal::Kill);
        }
        for a in &self.pipes {
            let _ = a.proc.signal(HeldSignal::Kill);
        }
        // What is left is killed and collected as the agents are dropped.
    }

    /// One pass; `false` when the holder is done. The node first, then the bounded rest.
    pub(crate) fn tick(&mut self) -> bool {
        if agora_platform::stop_requested() {
            self.end_agents();
            return false;
        }
        self.reap();
        self.publish();
        if self.departed.as_ref().is_some_and(ProcessWatch::ended) {
            self.departed = None;
            self.start_readers();
        }
        let served = self.serve_node();
        if served == Some(true) {
            return false;
        }
        let accepted = self.accept();
        // Idle (no node, nothing new): wait a tick. A node's read already waited one.
        if served.is_none() && accepted == 0 {
            std::thread::sleep(TICK);
        }
        true
    }

    /// Notes agents that ended, and keeps what an ended agent's terminal still holds.
    fn reap(&mut self) {
        for a in &mut self.pipes {
            if a.exit.is_none() {
                a.exit = a.proc.try_wait().unwrap_or(None);
                if a.exit.is_some() {
                    // Its input is closed with it: a line written to an ended agent is `Failed`, never queued and lost.
                    a.input = None;
                }
            }
        }
        let now = Instant::now();
        let node_may_read = self.departed.is_some();
        for a in &mut self.agents {
            if a.exit.is_none() {
                a.exit = a.term.try_wait().unwrap_or(None);
                if a.exit.is_none() && a.drain_to_end(now, node_may_read) {
                    a.exit = a.term.try_wait().unwrap_or(None);
                }
            }
            if a.exit.is_some() && !a.eof {
                // Reading goes on until the terminal ends; with a reader that has seen the end, take what it kept.
                if a.reader.as_ref().is_some_and(TerminalReader::ended) {
                    a.end_read();
                }
            }
        }
    }

    /// Takes at most [`MAX_ACCEPTS`] waiting connections; returns how many.
    fn accept(&mut self) -> usize {
        let mut n = 0;
        while n < MAX_ACCEPTS {
            match self.listener.accept() {
                Ok(stream) => {
                    n += 1;
                    self.admit_connection(stream);
                }
                Err(_) => break,
            }
        }
        n
    }

    /// The first rule on a new connection is who it is: before anything is read from it, a peer that is not the node holding the lock
    /// is closed without an answer.
    fn admit_connection(&mut self, mut stream: LocalStream) {
        let Ok(peer) = stream.peer() else { return };
        let Some(start) = (self.admit)(&peer) else {
            return;
        };
        if stream.set_read_timeout(Some(HELLO_WAIT)).is_err() {
            return;
        }
        let Ok(Some((payload, _))) = read_frame(&mut stream, HELLO_WAIT) else {
            return;
        };
        let Ok(Request::Hello { version }) = Request::decode(&payload) else {
            return;
        };
        // A node already attached keeps its place; one whose process ended has lost it.
        if let Some(n) = &self.node {
            if !n.watch.ended() {
                refuse(&mut stream, Refusal::Busy);
                return;
            }
            self.detach();
        }
        if !agora_node::holder_link::version_spoken(version) {
            refuse(&mut stream, Refusal::VersionMismatch);
            return;
        }
        let Ok(watch) = ProcessWatch::open(peer.pid, start) else {
            return;
        };
        let hello = Reply::HelloOk {
            version,
            holder_pid: self.pid,
            holder_start: self.start,
        };
        if write_frame(&mut stream, &hello.encode(), None).is_err()
            || stream.set_read_timeout(Some(TICK)).is_err()
        {
            return;
        }
        if stream.set_write_timeout(Some(WRITE_SLICE)).is_err() {
            return;
        }
        // A node that comes back takes over the terminals from the one before it: the holder reads none for it.
        self.departed = None;
        self.node = Some(Node {
            version,
            stream,
            watch,
            attached: false,
        });
    }

    /// Serves the node's next request. `None` when no node is connected; `Some(true)` when the holder is done.
    fn serve_node(&mut self) -> Option<bool> {
        let node = self.node.as_mut()?;
        if node.watch.ended() {
            self.detach();
            return Some(false);
        }
        let (payload, _) = match read_frame(&mut node.stream, TICK) {
            Ok(Some(f)) => f,
            Ok(None) => return Some(false),
            Err(_) => {
                self.detach();
                return Some(false);
            }
        };
        let Ok(request) = Request::decode(&payload) else {
            // A node that sends what this version cannot read is not served.
            self.detach();
            return Some(false);
        };
        Some(self.handle(request))
    }

    /// The node's link is gone. If its process ended too, every terminal is read into its buffer, so no agent blocks on a full
    /// terminal (decision 4). If it runs, it still has its copies of the terminals: it is watched, and the reads start when it ends.
    fn detach(&mut self) {
        let Some(node) = self.node.take() else {
            return;
        };
        if node.watch.ended() {
            self.start_readers();
        } else {
            self.departed = Some(node.watch);
        }
    }

    fn start_readers(&mut self) {
        for a in &mut self.agents {
            if a.reader.is_none() && !a.eof {
                a.reader = a.term.start_reader().ok();
            }
        }
    }

    fn send(&mut self, reply: &Reply, terminal: Option<&agora_platform::TerminalHandle>) -> bool {
        let Some(n) = self.node.as_mut() else {
            return false;
        };
        if write_frame(&mut n.stream, &reply.encode(), terminal).is_err() {
            self.detach();
            return false;
        }
        true
    }

    fn refuse(&mut self, code: Refusal) {
        self.send(
            &Reply::Refused {
                code,
                holder_version: LINK_VERSION,
            },
            None,
        );
    }

    /// Whether one more agent may be kept (#983): every agent it keeps, terminal or pipe, running or ended and not yet released, is an
    /// entry of the one `List` reply, which must fit a frame. A start past it is `Failed`, and the node runs that agent itself.
    fn has_room(&self) -> bool {
        self.agents.len() + self.pipes.len() < self.capacity
    }

    fn agent(&mut self, id: &str) -> Option<&mut Agent> {
        self.agents.iter_mut().find(|a| a.id == id)
    }

    /// Answers one request; `true` when the holder is done.
    fn handle(&mut self, request: Request) -> bool {
        let attached = self.node.as_ref().is_some_and(|n| n.attached);
        match request {
            Request::Hello { .. } => self.refuse(Refusal::BadRequest),
            Request::List => {
                let list = self
                    .agents
                    .iter()
                    .map(Agent::info)
                    .chain(self.pipes.iter().map(PipeAgent::info))
                    .collect();
                self.send(&Reply::Agents(list), None);
            }
            _ if !attached && !matches!(request, Request::Attach) => {
                self.refuse(Refusal::NotAttached);
            }
            Request::Attach if attached => self.refuse(Refusal::BadRequest),
            Request::Attach => return self.attach(),
            Request::Start(s) => self.start(&s),
            Request::StartPipe(_)
            | Request::AttachPipes
            | Request::PipeRead { .. }
            | Request::PipeWrite { .. }
                if self
                    .node
                    .as_ref()
                    .is_none_or(|n| n.version < LINK_VERSION_PIPES) =>
            {
                self.refuse(Refusal::BadRequest);
            }
            Request::StartPipe(s) => self.start_pipe(&s),
            Request::AttachPipes => return self.attach_pipes(),
            Request::PipeRead { agent } => self.pipe_read(&agent),
            Request::PipeWrite { agent, line } => self.pipe_write(&agent, line),
            Request::Resize { agent, cols, rows } => {
                // A pipe agent has no window to resize.
                if self.pipes.iter().any(|p| p.id == agent) {
                    self.refuse(Refusal::BadRequest);
                } else {
                    match self.agent(&agent) {
                        None => self.refuse(Refusal::UnknownAgent),
                        Some(a) => {
                            let r = a.term.resize(cols, rows);
                            self.answer(r);
                        }
                    }
                }
            }
            Request::Signal { agent, signal } => self.signal(&agent, signal),
            Request::Stop { agent, force } => self.signal(
                &agent,
                if force {
                    HeldSignal::Kill
                } else {
                    HeldSignal::Terminate
                },
            ),
            Request::Release { agent } => return self.release(&agent),
        }
        false
    }

    fn answer(&mut self, r: std::io::Result<()>) {
        match r {
            Ok(()) => {
                self.send(&Reply::Ok, None);
            }
            Err(_) => self.refuse(Refusal::Failed),
        }
    }

    fn pipe(&mut self, id: &str) -> Option<&mut PipeAgent> {
        self.pipes.iter_mut().find(|a| a.id == id)
    }

    fn signal(&mut self, agent: &str, signal: HeldSignal) {
        if let Some(p) = self.pipe(agent) {
            let r = p.proc.signal(signal);
            return self.answer(r);
        }
        match self.agent(agent) {
            None => self.refuse(Refusal::UnknownAgent),
            Some(a) => {
                let r = a.term.signal(signal);
                if r.is_ok() && matches!(signal, HeldSignal::Kill | HeldSignal::Terminate) {
                    a.ending.get_or_insert_with(Instant::now);
                }
                self.answer(r);
            }
        }
    }

    fn start(&mut self, s: &StartRequest) {
        // An id longer than `MAX_AGENT_ID` would make every later reply that names it (`Attach`, `List`) larger than a frame (#929).
        if !agent_id_fits(&s.agent) || s.argv.is_empty() {
            return self.refuse(Refusal::BadRequest);
        }
        if self.agent(&s.agent).is_some() || self.pipe(&s.agent).is_some() {
            return self.refuse(Refusal::AgentExists);
        }
        if !self.has_room() {
            return self.refuse(Refusal::Failed);
        }
        let config = PtyConfig {
            argv: s.argv.clone(),
            cwd: s.cwd.as_ref().map(PathBuf::from),
            env: s.env.clone(),
            cols: s.cols,
            rows: s.rows,
            limits: agora_platform::ProcessLimits {
                cpu_seconds: s.cpu_seconds,
                memory_bytes: s.memory_bytes,
            },
            sandbox: None,
            identity: None,
            env_remove: s.env_remove.clone(),
        };
        let term = match HeldTerminal::start(&config) {
            Ok(t) => t,
            Err(_) => return self.refuse(Refusal::Failed),
        };
        let mut term = term;
        let Ok(handle) = term.handle() else {
            // Never handed to the node: ended and reaped, so no process and no zombie is left behind.
            term.kill_and_reap();
            return self.refuse(Refusal::Failed);
        };
        let pid = term.pid();
        self.had_agents = true;
        self.agents.push(Agent {
            id: s.agent.clone(),
            term,
            reader: None,
            held: HeldBuffer::default(),
            exit: None,
            eof: false,
            ending: None,
        });
        self.send(
            &Reply::Started {
                agent: s.agent.clone(),
                pid,
            },
            Some(&handle),
        );
    }

    /// Hands every terminal and its buffer over: first each read is ended, and only then is anything sent (decision 4). A running
    /// agent's buffer is forgotten only once its frame was sent, so a link that breaks mid-way loses nothing.
    fn attach(&mut self) -> bool {
        for a in &mut self.agents {
            a.end_read();
            a.exit = a.exit.or_else(|| a.term.try_wait().unwrap_or(None));
            if a.exit.is_some() && !a.eof {
                // Nobody reads any more: what an ended agent left in its terminal is taken now.
                a.eof = a.term.drain(&mut a.held).unwrap_or(true);
            }
        }
        if let Some(n) = self.node.as_mut() {
            n.attached = true;
        }
        let count = self.agents.len();
        for i in 0..count {
            let a = &self.agents[i];
            let ended = a.exit.is_some();
            let terminal = if ended { None } else { a.term.handle().ok() };
            let reply = Reply::Terminal(TerminalInfo {
                info: a.info(),
                buffer_dropped: a.held.dropped(),
                buffer: a.held.clone().into_vec(),
            });
            if !self.send(&reply, terminal.as_ref()) {
                return false;
            }
            // Sent: a running agent's buffer is the node's now. An ended agent's stays until the node releases it.
            if !ended {
                self.agents[i].held = HeldBuffer::default();
            }
        }
        if !self.send(
            &Reply::AttachDone {
                count: count as u32,
            },
            None,
        ) {
            return false;
        }
        // A node that attaches and finds nothing left of what the holder held ends it (decision 2).
        self.had_agents && self.agents.is_empty() && self.pipes.is_empty()
    }

    /// ADR-0093: starts a program on pipes. Its output is read from now on, by a thread of its own, into a bounded buffer.
    fn start_pipe(&mut self, s: &StartPipeRequest) {
        if !agent_id_fits(&s.agent) || s.argv.is_empty() {
            return self.refuse(Refusal::BadRequest);
        }
        if self.agent(&s.agent).is_some() || self.pipe(&s.agent).is_some() {
            return self.refuse(Refusal::AgentExists);
        }
        if !self.has_room() {
            return self.refuse(Refusal::Failed);
        }
        let cwd = s.cwd.as_ref().map(PathBuf::from);
        let Ok(mut proc) = HeldProcess::start(&s.argv, cwd.as_deref(), &s.env) else {
            return self.refuse(Refusal::Failed);
        };
        let lines = PipeLines::default();
        let wired = proc.take_io().and_then(|(stdin, stdout)| {
            lines.spawn_reader(stdout).ok()?;
            LineWriter::spawn(stdin).ok()
        });
        let Some(input) = wired else {
            // Never handed to the node: ended and reaped, so no process and no zombie is left behind.
            proc.kill_and_reap();
            return self.refuse(Refusal::Failed);
        };
        let pid = proc.pid();
        self.had_agents = true;
        self.pipes.push(PipeAgent {
            id: s.agent.clone(),
            proc,
            lines,
            input: Some(input),
            exit: None,
        });
        self.send(
            &Reply::PipeStarted {
                agent: s.agent.clone(),
                pid,
            },
            None,
        );
    }

    /// ADR-0093: hands over every pipe agent with all it kept (what an earlier node had and what no node had).
    fn attach_pipes(&mut self) -> bool {
        let count = self.pipes.len();
        for i in 0..count {
            let a = &self.pipes[i];
            let (chunk, eof) = a.lines.replay();
            let data = PipeData {
                info: a.info(),
                dropped: chunk.dropped,
                eof,
                seen: chunk.seen,
                fresh: chunk.fresh,
            };
            if !self.send(&Reply::PipeData(data), None) {
                return false;
            }
        }
        self.send(
            &Reply::AttachDone {
                count: count as u32,
            },
            None,
        );
        false
    }

    /// ADR-0093: the output no node had. A reply that never reaches the node loses what it carried, as a terminal's does.
    fn pipe_read(&mut self, agent: &str) {
        let Some(a) = self.pipe(agent) else {
            return self.refuse(Refusal::UnknownAgent);
        };
        let (chunk, eof) = a.lines.take_fresh();
        let data = PipeData {
            info: a.info(),
            dropped: chunk.dropped,
            eof,
            seen: Vec::new(),
            fresh: chunk.fresh,
        };
        self.send(&Reply::PipeData(data), None);
    }

    /// ADR-0093: queues one line for the agent's input; a thread of its own writes it, so an agent that does not read never holds the
    /// holder. A full queue or a closed input is `Failed`.
    fn pipe_write(&mut self, agent: &str, line: Vec<u8>) {
        if line.len() >= MAX_FRAME {
            return self.refuse(Refusal::BadRequest);
        }
        let Some(a) = self.pipe(agent) else {
            return self.refuse(Refusal::UnknownAgent);
        };
        let queued = match &a.input {
            Some(w) => w.queue(line),
            None => Err(QueueError::Closed),
        };
        match queued {
            Ok(()) => {
                self.send(&Reply::Ok, None);
            }
            Err(_) => self.refuse(Refusal::Failed),
        }
    }

    /// The node took an ended agent: it is forgotten. The holder ends when that was its last.
    fn release(&mut self, agent: &str) -> bool {
        if let Some(i) = self.pipes.iter().position(|a| a.id == agent) {
            if self.pipes[i].exit.is_none() {
                self.refuse(Refusal::BadRequest);
                return false;
            }
            self.pipes.remove(i);
            self.send(&Reply::Ok, None);
            return self.agents.is_empty() && self.pipes.is_empty();
        }
        match self.agents.iter().position(|a| a.id == agent) {
            None => self.refuse(Refusal::UnknownAgent),
            Some(i) if self.agents[i].exit.is_none() => self.refuse(Refusal::BadRequest),
            Some(i) => {
                self.agents.remove(i);
                self.send(&Reply::Ok, None);
                return self.agents.is_empty() && self.pipes.is_empty();
            }
        }
        false
    }
}

fn refuse(stream: &mut LocalStream, code: Refusal) {
    let reply = Reply::Refused {
        code,
        holder_version: LINK_VERSION,
    };
    let _ = write_frame(stream, &reply.encode(), None);
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn agent_with_output() -> Agent {
        let config = PtyConfig {
            argv: vec!["sh".into(), "-c".into(), "echo up; sleep 600".into()],
            cwd: None,
            env: Vec::new(),
            cols: 80,
            rows: 24,
            limits: agora_platform::ProcessLimits {
                cpu_seconds: None,
                memory_bytes: None,
            },
            sandbox: None,
            identity: None,
            env_remove: Vec::new(),
        };
        let term = HeldTerminal::start(&config).unwrap();
        Agent {
            id: "a".into(),
            term,
            reader: None,
            held: HeldBuffer::default(),
            exit: None,
            eof: false,
            ending: None,
        }
    }

    /// An agent being ended whose terminal nobody reads is read by the holder (so macOS lets it exit); one a node may read, one that is
    /// not being ended, and one still within its grace are not.
    #[test]
    fn an_ending_agent_nobody_reads_is_drained_and_no_other_is() {
        let mut a = agent_with_output();
        std::thread::sleep(Duration::from_millis(300));
        let t0 = Instant::now();
        let later = t0 + ENDING_GRACE * 2;
        // Not being ended: never read (the node reads it).
        assert!(!a.drain_to_end(later, false));
        a.ending = Some(t0);
        // Within the grace: the node may still read it.
        assert!(!a.drain_to_end(t0, false));
        // A node whose link dropped still holds a copy.
        assert!(!a.drain_to_end(later, true));
        assert!(a.held.clone().into_vec().is_empty(), "output was taken");
        // Ending, past the grace, nobody to read: read.
        assert!(a.drain_to_end(later, false));
        assert!(String::from_utf8_lossy(&a.held.clone().into_vec()).contains("up"));
        // A reaped agent is left alone.
        let _ = a.term.signal(HeldSignal::Kill);
        std::thread::sleep(Duration::from_millis(200));
        a.exit = a.term.try_wait().unwrap_or(None);
        assert!(a.exit.is_some());
        assert!(!a.drain_to_end(later, false));
    }

    /// #983: a holder keeps no more agents than one `List` reply can name. A `Start` or `StartPipe` past its capacity (terminal and
    /// pipe agents together) is refused with `Failed`, which the node takes as "run it yourself", and nothing is started; `List`
    /// still answers with what it keeps.
    #[test]
    fn a_full_holder_refuses_another_start_and_its_list_still_fits() {
        let home = std::env::temp_dir().join(format!("agora-full-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let mut holder = Holder::open(&home, Box::new(|_| None)).unwrap();
        let mut node = agora_platform::local_connect(&socket_path(&home)).unwrap();
        let stream = holder.listener.accept().unwrap();
        let me = std::process::id();
        holder.node = Some(Node {
            stream,
            version: LINK_VERSION_PIPES,
            watch: ProcessWatch::open(me, agora_platform::process_start_time(me).unwrap()).unwrap(),
            attached: true,
        });
        holder.capacity = 2;
        let wait = Duration::from_secs(5);
        let reply = |node: &mut LocalStream| {
            let (payload, _) = read_frame(node, wait).unwrap().unwrap();
            Reply::decode(&payload).unwrap()
        };
        let argv = || vec!["sleep".to_string(), "600".to_string()];
        let start = |agent: &str| {
            Request::Start(StartRequest {
                agent: agent.into(),
                argv: argv(),
                cols: 80,
                rows: 24,
                ..StartRequest::default()
            })
        };
        let start_pipe = |agent: &str| {
            Request::StartPipe(StartPipeRequest {
                agent: agent.into(),
                argv: argv(),
                cwd: None,
                env: Vec::new(),
            })
        };
        holder.handle(start("t1"));
        assert!(matches!(reply(&mut node), Reply::Started { .. }));
        holder.handle(start_pipe("p1"));
        assert!(matches!(reply(&mut node), Reply::PipeStarted { .. }));
        for full in [start("t2"), start_pipe("p2")] {
            holder.handle(full);
            assert!(
                matches!(
                    reply(&mut node),
                    Reply::Refused {
                        code: Refusal::Failed,
                        ..
                    }
                ),
                "a start past the capacity was not refused"
            );
        }
        assert_eq!((holder.agents.len(), holder.pipes.len()), (1, 1));
        holder.handle(Request::List);
        let Reply::Agents(list) = reply(&mut node) else {
            panic!("no list")
        };
        let ids: Vec<&str> = list.iter().map(|a| a.agent.as_str()).collect();
        assert_eq!(ids, ["t1", "p1"]);
        // An id already held is still `AgentExists`, not "full": the node ends that earlier run and starts again.
        holder.handle(start("t1"));
        assert!(matches!(
            reply(&mut node),
            Reply::Refused {
                code: Refusal::AgentExists,
                ..
            }
        ));
        for a in &mut holder.agents {
            a.term.kill_and_reap();
        }
        for p in &mut holder.pipes {
            p.proc.kill_and_reap();
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The lock names a supervisor, which runs the node as its child (ADR-0062 item 8): the node is admitted, the supervisor itself is,
    /// and neither a stranger nor the node's own child (an agent, a confined one included) is.
    #[test]
    fn the_lock_holder_and_its_direct_child_are_admitted_and_no_one_else() {
        let dir = std::env::temp_dir().join(format!("agora-admit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lock = dir.join("node.lock");
        let me = std::process::id();
        let mine = crate::frontdoor::LockInfo::of(me).unwrap();
        std::fs::write(&lock, mine.text()).unwrap();
        let peer = |pid: u32| Peer {
            pid,
            same_user: true,
        };

        // This process stands for the supervisor. Its node is an `agora` process (here `sleep` under that name); a stranger it started
        // is not named so, and what the node started is the node's child, never the lock holder's.
        let node_exe = dir.join("agora-admit-node");
        std::fs::write(&node_exe, "#!/bin/sh\nsleep 5\n").unwrap();
        std::fs::set_permissions(
            &node_exe,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        let mut node = std::process::Command::new(&node_exe)
            .arg("5")
            .spawn()
            .unwrap();
        let mut stranger = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let mut grandchild = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("{} 5 & wait", node_exe.display()))
            .spawn()
            .unwrap();
        let node_start = agora_platform::process_start_time(node.id()).unwrap();
        assert_eq!(admit_by_lock(&lock, &peer(me)), Some(mine.start));
        // Where a script is named after its interpreter (macOS) the stand-in cannot be named `agora`: the positive case is then
        // only covered on Linux, and the refusals below hold either way.
        std::thread::sleep(std::time::Duration::from_millis(200));
        if agora_platform::process_name(node.id()).is_ok_and(|n| n.starts_with("agora")) {
            assert_eq!(admit_by_lock(&lock, &peer(node.id())), Some(node_start));
        }
        assert_eq!(
            admit_by_lock(&lock, &peer(stranger.id())),
            None,
            "a child of the lock holder that is not the node was admitted"
        );
        std::thread::sleep(std::time::Duration::from_millis(300));
        let agents = agora_platform::process_children(grandchild.id()).unwrap();
        assert!(!agents.is_empty(), "the shell started nothing");
        for agent in agents {
            assert_eq!(
                admit_by_lock(&lock, &peer(agent)),
                None,
                "a grandchild of the lock holder was admitted"
            );
        }
        // Another user, however it is related.
        let other_user = Peer {
            pid: node.id(),
            same_user: false,
        };
        assert_eq!(admit_by_lock(&lock, &other_user), None);
        // A process that is not related to the lock holder at all (pid 1 is nobody's child but its own parent's).
        assert_eq!(admit_by_lock(&lock, &peer(1)), None);
        // A lock that names a process that is gone admits no child of it.
        std::fs::write(&lock, format!("{} {} test\n", me, mine.start + 1)).unwrap();
        assert_eq!(admit_by_lock(&lock, &peer(node.id())), None);
        for child in [&mut node, &mut stranger, &mut grandchild] {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
