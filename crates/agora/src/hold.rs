//! `agora hold`: the terminal holder (ADR-0079), the process that owns the agents' terminals so they outlive their node.
//!
//! The holder never decides anything. It starts, resizes, signals and stops an agent only when its node asks, keeps the terminals,
//! reads them into a bounded buffer while no node is attached, and keeps the exit status and unread output of agents that ended with
//! no node there until a node takes them. It serves one node at a time, and only the process that holds this home's node lock, by
//! pid and start time (ADR-0062 item 8). The link is `agora_node::holder_link`; `docs/specs/holder-link.md` describes it.
//!
//! One thread does everything, so the process that starts each agent lives as long as the holder (on Linux the agent's first process
//! gets `SIGHUP` when it ends, which the kernel ties to the thread that forked). The node starts it and starts its agents through it
//! (W2 step 2, `agora_node::holder`). It writes `run/hold.pid` (its pid, start time and how many agents run), which `agora down` and
//! `agora status` read, and on `SIGTERM` (`agora down --agents`) it ends its agents and itself.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use agora_i18n::key;
use agora_node::holder_link::{
    read_frame, socket_path, write_frame, AgentInfo, AgentState, Refusal, Reply, Request,
    StartRequest, TerminalInfo, LINK_VERSION, WRITE_SLICE,
};
use agora_platform::{
    ExitStatus, HeldBuffer, HeldSignal, HeldTerminal, LocalListener, LocalStream, Peer,
    ProcessWatch, PtyConfig, TerminalReader,
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
            departed: None,
            had_agents: false,
            pid,
            start,
            home: home.to_path_buf(),
            published: None,
        })
    }

    /// Serves until nothing is left to keep: a holder that held agents and has none left, after a node took the last.
    pub(crate) fn run(mut self) {
        while self.tick() {}
        let _ = std::fs::remove_file(info_path(&self.home));
    }

    /// Says (in `run/hold.pid`) how many agents run, when that changed. A failed write is only a stale count.
    fn publish(&mut self) {
        let running = self.agents.iter().filter(|a| a.exit.is_none()).count();
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
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end {
            self.reap();
            if self.agents.iter().all(|a| a.exit.is_some()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        for a in &self.agents {
            let _ = a.term.signal(HeldSignal::Kill);
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
        if stream.set_write_timeout(Some(WRITE_SLICE)).is_err() {
            return;
        }
        // A node that comes back takes over the terminals from the one before it: the holder reads none for it.
        self.departed = None;
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

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

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
        assert_eq!(admit_by_lock(&lock, &peer(node.id())), Some(node_start));
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
