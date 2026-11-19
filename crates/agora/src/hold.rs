//! `agora hold`: the terminal holder (ADR-0079), the process that owns the agents' terminals so they outlive their node.
//!
//! The holder never decides anything. It starts, resizes, signals and stops an agent only when its node asks, keeps the terminals,
//! reads them into a bounded buffer while no node is attached, and keeps the exit status and unread output of agents that ended with
//! no node there until a node takes them. It serves one node at a time, and only the process that holds this home's node lock, by
//! pid and start time (ADR-0062 item 8). The link is `agora_node::holder_link`; `docs/specs/holder-link.md` describes it.
//!
//! One thread does everything, so the process that starts each agent lives as long as the holder (on Linux the agent's first process
//! gets `SIGHUP` when it ends, which the kernel ties to the thread that forked). Nothing here is wired into the node yet (W2 step 1).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use agora_i18n::key;
use agora_node::holder_link::{
    read_frame, socket_path, write_frame, AgentInfo, AgentState, Refusal, Reply, Request,
    StartRequest, TerminalInfo, LINK_VERSION,
};
use agora_platform::{
    ExitStatus, HeldBuffer, HeldSignal, HeldTerminal, LocalListener, LocalStream, Peer,
    ProcessWatch, PtyConfig, TerminalReader,
};

use crate::frontdoor::{read_lock, Role};
use crate::{home, Opts, Say};

/// How often the holder looks for a node's end, an ended agent and a new connection.
const TICK: Duration = Duration::from_millis(50);

/// How long a peer that passed the pid check may take to say hello.
const HELLO_WAIT: Duration = Duration::from_secs(1);

/// `agora hold`.
pub(crate) fn hold(say: &Say, opts: &Opts) -> ExitCode {
    let home = opts.home.clone().unwrap_or_else(home);
    // Its own session: not ended with the terminal or the process group that started it (decision 2). A failure leaves it where it is.
    let _ = agora_platform::start_new_session();
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

/// Whether `peer` is the node holding the role lock now: its pid is the lock's, and its start time (read now) is the lock's, so a
/// pid another process got after the node ended is not admitted. Returns that start time.
pub(crate) fn admit_by_lock(lock: &Path, peer: &Peer) -> Option<u64> {
    if !peer.same_user {
        return None;
    }
    let l = read_lock(lock)?;
    let start = agora_platform::process_start_time(peer.pid)?;
    (l.pid == peer.pid && l.start == start).then_some(start)
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
}

impl Agent {
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

struct Node {
    stream: LocalStream,
    watch: ProcessWatch,
    attached: bool,
}

pub(crate) struct Holder {
    listener: LocalListener,
    admit: Admit,
    agents: Vec<Agent>,
    node: Option<Node>,
    /// An agent was ever started: a holder that never had one waits for its first.
    had_agents: bool,
    pid: u32,
    start: u64,
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
            node: None,
            had_agents: false,
            pid,
            start,
        })
    }

    /// Serves until nothing is left to keep: a holder that held agents and has none left, after a node took the last.
    pub(crate) fn run(mut self) {
        while self.tick() {}
    }

    /// One pass; `false` when the holder is done.
    pub(crate) fn tick(&mut self) -> bool {
        self.reap();
        self.accept();
        if let Some(done) = self.serve_node() {
            if done {
                return false;
            }
        } else {
            std::thread::sleep(TICK);
        }
        true
    }

    /// Notes agents that ended, and keeps what an ended agent's terminal still holds.
    fn reap(&mut self) {
        for a in &mut self.agents {
            if a.exit.is_none() {
                a.exit = a.term.try_wait().unwrap_or(None);
            }
            if a.exit.is_some() && !a.eof {
                // Reading goes on until the terminal ends; with a reader that has seen the end, take what it kept.
                if a.reader.as_ref().is_some_and(TerminalReader::ended) {
                    a.end_read();
                }
            }
        }
    }

    fn accept(&mut self) {
        loop {
            match self.listener.accept() {
                Ok(stream) => self.admit_connection(stream),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return,
                Err(_) => return,
            }
        }
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
        let Ok(Some((payload, _))) = read_frame(&mut stream) else {
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
        if version != LINK_VERSION {
            refuse(&mut stream, Refusal::VersionMismatch);
            return;
        }
        let Ok(watch) = ProcessWatch::open(peer.pid, start) else {
            return;
        };
        let hello = Reply::HelloOk {
            version: LINK_VERSION,
            holder_pid: self.pid,
            holder_start: self.start,
        };
        if write_frame(&mut stream, &hello.encode(), None).is_err()
            || stream.set_read_timeout(Some(TICK)).is_err()
        {
            return;
        }
        self.node = Some(Node {
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
        let (payload, _) = match read_frame(&mut node.stream) {
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

    /// The node is gone: every terminal is read into its buffer again, so no agent blocks on a full terminal (decision 4).
    fn detach(&mut self) {
        self.node = None;
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

    fn agent(&mut self, id: &str) -> Option<&mut Agent> {
        self.agents.iter_mut().find(|a| a.id == id)
    }

    /// Answers one request; `true` when the holder is done.
    fn handle(&mut self, request: Request) -> bool {
        let attached = self.node.as_ref().is_some_and(|n| n.attached);
        match request {
            Request::Hello { .. } => self.refuse(Refusal::BadRequest),
            Request::List => {
                let list = self.agents.iter().map(Agent::info).collect();
                self.send(&Reply::Agents(list), None);
            }
            _ if !attached && !matches!(request, Request::Attach) => {
                self.refuse(Refusal::NotAttached);
            }
            Request::Attach if attached => self.refuse(Refusal::BadRequest),
            Request::Attach => return self.attach(),
            Request::Start(s) => self.start(&s),
            Request::Resize { agent, cols, rows } => match self.agent(&agent) {
                None => self.refuse(Refusal::UnknownAgent),
                Some(a) => {
                    let r = a.term.resize(cols, rows);
                    self.answer(r);
                }
            },
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

    fn signal(&mut self, agent: &str, signal: HeldSignal) {
        match self.agent(agent) {
            None => self.refuse(Refusal::UnknownAgent),
            Some(a) => {
                let r = a.term.signal(signal);
                self.answer(r);
            }
        }
    }

    fn start(&mut self, s: &StartRequest) {
        if s.agent.is_empty() || s.argv.is_empty() {
            return self.refuse(Refusal::BadRequest);
        }
        if self.agent(&s.agent).is_some() {
            return self.refuse(Refusal::AgentExists);
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
        let Ok(handle) = term.handle() else {
            let _ = term.signal(HeldSignal::Kill);
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
        });
        self.send(
            &Reply::Started {
                agent: s.agent.clone(),
                pid,
            },
            Some(&handle),
        );
    }

    /// Hands every terminal and its buffer over: first each read is ended, and only then is anything sent (decision 4).
    fn attach(&mut self) -> bool {
        let mut frames = Vec::new();
        for a in &mut self.agents {
            a.end_read();
            a.exit = a.exit.or_else(|| a.term.try_wait().unwrap_or(None));
            if a.exit.is_some() && !a.eof {
                // Nobody reads any more: what an ended agent left in its terminal is taken now.
                a.eof = a.term.drain(&mut a.held).unwrap_or(true);
            }
            let ended = a.exit.is_some();
            let buffer_dropped = a.held.dropped();
            // A running agent's buffer goes to the node. An ended agent's stays until the node releases it.
            let held = if ended {
                a.held.clone()
            } else {
                std::mem::take(&mut a.held)
            };
            let terminal = if ended { None } else { a.term.handle().ok() };
            frames.push((
                Reply::Terminal(TerminalInfo {
                    info: a.info(),
                    buffer_dropped,
                    buffer: held.into_vec(),
                }),
                terminal,
            ));
        }
        let count = frames.len() as u32;
        if let Some(n) = self.node.as_mut() {
            n.attached = true;
        }
        for (reply, terminal) in &frames {
            if !self.send(reply, terminal.as_ref()) {
                return false;
            }
        }
        if !self.send(&Reply::AttachDone { count }, None) {
            return false;
        }
        // A node that attaches and finds nothing left of what the holder held ends it (decision 2).
        self.had_agents && self.agents.is_empty()
    }

    /// The node took an ended agent: it is forgotten. The holder ends when that was its last.
    fn release(&mut self, agent: &str) -> bool {
        match self.agents.iter().position(|a| a.id == agent) {
            None => self.refuse(Refusal::UnknownAgent),
            Some(i) if self.agents[i].exit.is_none() => self.refuse(Refusal::BadRequest),
            Some(i) => {
                self.agents.remove(i);
                self.send(&Reply::Ok, None);
                return self.agents.is_empty();
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
