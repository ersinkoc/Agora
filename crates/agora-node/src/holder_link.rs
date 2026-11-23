//! ADR-0079 decision 5: the private link between a node and its terminal holder (`agora hold`), and the node's client for it.
//!
//! Not a protocol message (`schema/` is for the wire between nodes, hubs and windows): a small versioned format private to Agora, over
//! a local socket. `docs/specs/holder-link.md` is its specification; `tests/fixtures/holder-link/` holds golden fixtures, added and
//! never edited.
//!
//! Every message is one frame: `length: u32 BE` (of what follows), `tag: u8`, the body. The frame, `Hello` (tag 1) and `Refused`
//! (tag 65) are frozen for good, so a node and a holder of different versions can always tell each other that they differ. A terminal
//! (Unix: its PTY master, by `SCM_RIGHTS`) travels with the frame that names it.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use agora_platform::{HeldSignal, LocalStream, ReadTimeout, TerminalHandle};

/// The version of the holder link this build speaks. A holder and a node that differ do nothing together (decision 5).
pub const LINK_VERSION: u16 = 1;

/// ADR-0093: the version that adds the pipe agents (tags 9 to 12 and 71, 72). A holder speaks both; a node speaks the newest the holder
/// does, so a holder built before this version (which closes the link on a tag it does not know) is never sent one, and a node built
/// before it keeps working with this holder (it speaks version 1, and the holder sends it none of the new tags).
pub const LINK_VERSION_PIPES: u16 = 2;

/// Whether this build speaks link version `v`.
pub fn version_spoken(v: u16) -> bool {
    v == LINK_VERSION || v == LINK_VERSION_PIPES
}

/// The largest frame either side accepts: a terminal's whole buffer (1 MiB, [`agora_platform::HELD_BUFFER`]) and generous room for the rest.
pub const MAX_FRAME: usize = 4 << 20;

/// The longest agent id (in bytes) the holder takes in `Start` and `StartPipe` (#929); a longer or empty one is `Refused(BadRequest)`.
/// The node's ids are `agent-<n>` (at most 26 bytes), so this is room to spare, and it keeps every reply that names an agent within
/// [`MAX_FRAME`]: a `Terminal` or `PipeData` frame is one id and at most one buffer, and a `List` entry costs at most
/// [`MAX_LIST_ENTRY`] bytes. A node never sends a longer id: it runs such an agent itself ([`agent_id_fits`]).
pub const MAX_AGENT_ID: usize = 256;

/// The most one agent costs in a `List` (`Agents`) reply: its id (length and bytes), pid, and state (a tag and two optional `i32`s).
pub const MAX_LIST_ENTRY: usize = 4 + MAX_AGENT_ID + 4 + 1 + 5 + 5;

/// The most agents one holder keeps, terminal and pipe agents together, ended ones not yet released included (#983): every one is an
/// entry of the one `List` (`Agents`) reply, so this is as many as fit a frame (15,251). A `Start` or `StartPipe` past it is
/// `Refused(Failed)` and nothing starts; the node then runs the agent itself, as for any start the holder cannot make.
pub const MAX_HELD_AGENTS: usize = (MAX_FRAME - 1 - 4) / MAX_LIST_ENTRY;

// A `List` reply (tag, count, and every entry at its longest) for a full holder fits a frame.
const _: () = assert!(1 + 4 + MAX_HELD_AGENTS * MAX_LIST_ENTRY <= MAX_FRAME);

// A `Terminal` frame (tag, entry, buffer-dropped flag, buffer length and the whole buffer) and a `PipeData` frame (tag, entry, dropped,
// eof, two line counts, and everything kept, whose bound is on its encoded size) fit a frame with any id the holder takes.
const _: () = assert!(1 + MAX_LIST_ENTRY + 1 + 4 + agora_platform::HELD_BUFFER <= MAX_FRAME);
const _: () =
    assert!(1 + MAX_LIST_ENTRY + 8 + 1 + 4 + 4 + agora_platform::HELD_BUFFER <= MAX_FRAME);

/// Whether `id` may be sent to the holder as an agent's id: not empty and at most [`MAX_AGENT_ID`] bytes.
pub fn agent_id_fits(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_AGENT_ID
}

/// How long a peer may take to finish a frame it began.
pub const FRAME_TIMEOUT: Duration = Duration::from_secs(5);

/// The write timeout to set on a link's stream: each blocked send waits this long, and [`write_frame`] gives up for good once
/// [`FRAME_TIMEOUT`] has passed since the frame began (a send that moved some bytes returns early, so the timeout alone would restart).
pub const WRITE_SLICE: Duration = Duration::from_secs(1);

/// Where the holder of the Agora home `home` listens (a `0700` folder, a `0600` socket).
pub fn socket_path(home: &Path) -> PathBuf {
    home.join("run").join("hold.sock")
}

/// What went wrong on the link.
#[derive(Debug)]
pub enum LinkError {
    Io(io::Error),
    /// The holder said no, and why; `holder_version` is its own link version.
    Refused(Refusal, u16),
    /// The peer sent something this version does not understand.
    Malformed(&'static str),
    /// The peer closed the link.
    Closed,
}

impl From<io::Error> for LinkError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "holder link: {e}"),
            Self::Refused(r, v) => write!(f, "holder link: refused ({r:?}, holder version {v})"),
            Self::Malformed(w) => write!(f, "holder link: malformed {w}"),
            Self::Closed => write!(f, "holder link: closed"),
        }
    }
}

impl std::error::Error for LinkError {}

/// Why the holder refused a request. Stable numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    VersionMismatch = 1,
    /// Another node is attached.
    Busy = 2,
    /// The request needs `Attach` first.
    NotAttached = 3,
    UnknownAgent = 4,
    AgentExists = 5,
    BadRequest = 6,
    /// The system refused (the program did not start, the signal was not sent), or the holder already keeps [`MAX_HELD_AGENTS`].
    Failed = 7,
}

impl Refusal {
    fn from_u8(n: u8) -> Option<Self> {
        Some(match n {
            1 => Self::VersionMismatch,
            2 => Self::Busy,
            3 => Self::NotAttached,
            4 => Self::UnknownAgent,
            5 => Self::AgentExists,
            6 => Self::BadRequest,
            7 => Self::Failed,
            _ => return None,
        })
    }
}

/// The launch the node already decided (decision 5): the holder adds and widens nothing. The confinement a node builds (`Sandbox`,
/// the agent identity) is not carried yet: step 2 adds it with a new message, never by changing this one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StartRequest {
    /// The node's own name for the agent (its ledger id); the holder never interprets it.
    pub agent: String,
    pub argv: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    pub env_remove: Vec<String>,
    pub cols: u16,
    pub rows: u16,
    pub cpu_seconds: Option<u64>,
    pub memory_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Running,
    /// The agent ended: its exit code, or the signal that ended it.
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInfo {
    pub agent: String,
    pub pid: u32,
    pub state: AgentState,
}

/// One held terminal as `Attach` reports it: what it is and the output kept while no node was attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalInfo {
    pub info: AgentInfo,
    /// Older output was dropped to stay within the bound (the buffer may start inside an escape sequence).
    pub buffer_dropped: bool,
    pub buffer: Vec<u8>,
}

/// ADR-0093: a program the node wants run on pipes (a structured agent: an adapter or an ACP bridge). Like [`StartRequest`] the launch
/// is the node's, complete: the holder adds nothing, and passes on exactly this environment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StartPipeRequest {
    pub agent: String,
    pub argv: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
}

/// ADR-0093: what a pipe agent wrote that the node has to take: lines (the holder never reads their content), what was lost, and
/// whether more can come.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeData {
    pub info: AgentInfo,
    /// Bytes dropped before any node saw them since the last hand-over (the oldest were pushed out of the bounded buffer).
    pub dropped: u64,
    /// The agent's output ended: every process that held it closed it. With no lines left, nothing more will come.
    pub eof: bool,
    /// Lines an earlier node was given, oldest first (only in a replay at attach): what that node may have been waiting on.
    pub seen: Vec<Vec<u8>>,
    /// Lines no node had, oldest first.
    pub fresh: Vec<Vec<u8>>,
}

/// One pipe agent as `AttachPipes` reports it.
pub type HeldPipe = PipeData;

/// What a node sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Hello {
        version: u16,
    },
    /// Hand me the terminals: the holder ends its own reads, then sends each terminal and its buffer.
    Attach,
    Start(StartRequest),
    Resize {
        agent: String,
        cols: u16,
        rows: u16,
    },
    Signal {
        agent: String,
        signal: HeldSignal,
    },
    /// End the agent's process group (`SIGTERM`, or `SIGKILL` when `force`).
    Stop {
        agent: String,
        force: bool,
    },
    List,
    /// The node has taken an ended agent's status and output: the holder forgets it.
    Release {
        agent: String,
    },
    /// ADR-0093: start a program on pipes. Answered `PipeStarted`.
    StartPipe(StartPipeRequest),
    /// ADR-0093: hand over every pipe agent with everything kept of its output (a replay); one `PipeData` each, then `AttachDone`.
    AttachPipes,
    /// ADR-0093: the pipe agent's output no node had yet. Answered `PipeData`.
    PipeRead {
        agent: String,
    },
    /// ADR-0093: one line (without its newline) for the pipe agent's input.
    PipeWrite {
        agent: String,
        line: Vec<u8>,
    },
}

/// What the holder answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    HelloOk {
        version: u16,
        holder_pid: u32,
        holder_start: u64,
    },
    Refused {
        code: Refusal,
        holder_version: u16,
    },
    Ok,
    /// An agent started; its terminal travels with this frame.
    Started {
        agent: String,
        pid: u32,
    },
    Agents(Vec<AgentInfo>),
    /// One held terminal; a running agent's terminal travels with this frame.
    Terminal(TerminalInfo),
    AttachDone {
        count: u32,
    },
    /// ADR-0093: a pipe agent started.
    PipeStarted {
        agent: String,
        pid: u32,
    },
    /// ADR-0093: a pipe agent's output.
    PipeData(PipeData),
}

// ----- encoding -----

const T_HELLO: u8 = 1;
const T_ATTACH: u8 = 2;
const T_START: u8 = 3;
const T_RESIZE: u8 = 4;
const T_SIGNAL: u8 = 5;
const T_STOP: u8 = 6;
const T_LIST: u8 = 7;
const T_RELEASE: u8 = 8;
const T_START_PIPE: u8 = 9;
const T_ATTACH_PIPES: u8 = 10;
const T_PIPE_READ: u8 = 11;
const T_PIPE_WRITE: u8 = 12;
const T_HELLO_OK: u8 = 64;
const T_REFUSED: u8 = 65;
const T_OK: u8 = 66;
const T_STARTED: u8 = 67;
const T_AGENTS: u8 = 68;
const T_TERMINAL: u8 = 69;
const T_ATTACH_DONE: u8 = 70;
const T_PIPE_STARTED: u8 = 71;
const T_PIPE_DATA: u8 = 72;

#[derive(Default)]
struct Enc(Vec<u8>);

impl Enc {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }
    fn bytes(&mut self, v: &[u8]) {
        // A length that does not fit is refused by `MAX_FRAME` at the frame; saturate rather than wrap.
        self.u32(u32::try_from(v.len()).unwrap_or(u32::MAX));
        self.0.extend_from_slice(v);
    }
    fn str(&mut self, v: &str) {
        self.bytes(v.as_bytes());
    }
    fn opt_str(&mut self, v: &Option<String>) {
        match v {
            Some(s) => {
                self.u8(1);
                self.str(s);
            }
            None => self.u8(0),
        }
    }
    fn opt_u64(&mut self, v: Option<u64>) {
        match v {
            Some(n) => {
                self.u8(1);
                self.u64(n);
            }
            None => self.u8(0),
        }
    }
    fn opt_i32(&mut self, v: Option<i32>) {
        match v {
            Some(n) => {
                self.u8(1);
                self.i32(n);
            }
            None => self.u8(0),
        }
    }
    fn state(&mut self, s: AgentState) {
        match s {
            AgentState::Running => self.u8(0),
            AgentState::Exited { code, signal } => {
                self.u8(1);
                self.opt_i32(code);
                self.opt_i32(signal);
            }
        }
    }
    fn info(&mut self, i: &AgentInfo) {
        self.str(&i.agent);
        self.u32(i.pid);
        self.state(i.state);
    }
    fn lines(&mut self, lines: &[Vec<u8>]) {
        self.u32(u32::try_from(lines.len()).unwrap_or(u32::MAX));
        for l in lines {
            self.bytes(l);
        }
    }
}

struct Dec<'a>(&'a [u8]);

impl<'a> Dec<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], LinkError> {
        if self.0.len() < n {
            return Err(LinkError::Malformed("frame (too short)"));
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, LinkError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, LinkError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, LinkError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Result<u64, LinkError> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_be_bytes(a))
    }
    fn i32(&mut self) -> Result<i32, LinkError> {
        let b = self.take(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn bool(&mut self) -> Result<bool, LinkError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(LinkError::Malformed("flag")),
        }
    }
    fn bytes(&mut self) -> Result<&'a [u8], LinkError> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    fn str(&mut self) -> Result<String, LinkError> {
        String::from_utf8(self.bytes()?.to_vec()).map_err(|_| LinkError::Malformed("text"))
    }
    fn opt_str(&mut self) -> Result<Option<String>, LinkError> {
        if self.bool()? {
            Ok(Some(self.str()?))
        } else {
            Ok(None)
        }
    }
    fn opt_u64(&mut self) -> Result<Option<u64>, LinkError> {
        if self.bool()? {
            Ok(Some(self.u64()?))
        } else {
            Ok(None)
        }
    }
    fn opt_i32(&mut self) -> Result<Option<i32>, LinkError> {
        if self.bool()? {
            Ok(Some(self.i32()?))
        } else {
            Ok(None)
        }
    }
    /// A count that cannot be larger than what is left (every item takes at least one byte), so a lie cannot make a huge list.
    fn count(&mut self) -> Result<usize, LinkError> {
        let n = self.u32()? as usize;
        if n > self.0.len() {
            return Err(LinkError::Malformed("count"));
        }
        Ok(n)
    }
    fn state(&mut self) -> Result<AgentState, LinkError> {
        match self.u8()? {
            0 => Ok(AgentState::Running),
            1 => Ok(AgentState::Exited {
                code: self.opt_i32()?,
                signal: self.opt_i32()?,
            }),
            _ => Err(LinkError::Malformed("state")),
        }
    }
    fn info(&mut self) -> Result<AgentInfo, LinkError> {
        Ok(AgentInfo {
            agent: self.str()?,
            pid: self.u32()?,
            state: self.state()?,
        })
    }
    fn lines(&mut self) -> Result<Vec<Vec<u8>>, LinkError> {
        let mut v = Vec::new();
        for _ in 0..self.count()? {
            v.push(self.bytes()?.to_vec());
        }
        Ok(v)
    }
    fn done(&self) -> Result<(), LinkError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(LinkError::Malformed("frame (trailing bytes)"))
        }
    }
}

fn signal_code(s: HeldSignal) -> u8 {
    match s {
        HeldSignal::Interrupt => 1,
        HeldSignal::Terminate => 2,
        HeldSignal::Kill => 3,
        HeldSignal::Hangup => 4,
    }
}

fn signal_from(n: u8) -> Result<HeldSignal, LinkError> {
    Ok(match n {
        1 => HeldSignal::Interrupt,
        2 => HeldSignal::Terminate,
        3 => HeldSignal::Kill,
        4 => HeldSignal::Hangup,
        _ => return Err(LinkError::Malformed("signal")),
    })
}

impl Request {
    /// The frame's payload: its tag and body (no length).
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Enc::default();
        match self {
            Self::Hello { version } => {
                e.u8(T_HELLO);
                e.u16(*version);
            }
            Self::Attach => e.u8(T_ATTACH),
            Self::Start(s) => {
                e.u8(T_START);
                e.str(&s.agent);
                e.u32(u32::try_from(s.argv.len()).unwrap_or(u32::MAX));
                for a in &s.argv {
                    e.str(a);
                }
                e.opt_str(&s.cwd);
                e.u32(u32::try_from(s.env.len()).unwrap_or(u32::MAX));
                for (k, v) in &s.env {
                    e.str(k);
                    e.str(v);
                }
                e.u32(u32::try_from(s.env_remove.len()).unwrap_or(u32::MAX));
                for r in &s.env_remove {
                    e.str(r);
                }
                e.u16(s.cols);
                e.u16(s.rows);
                e.opt_u64(s.cpu_seconds);
                e.opt_u64(s.memory_bytes);
            }
            Self::Resize { agent, cols, rows } => {
                e.u8(T_RESIZE);
                e.str(agent);
                e.u16(*cols);
                e.u16(*rows);
            }
            Self::Signal { agent, signal } => {
                e.u8(T_SIGNAL);
                e.str(agent);
                e.u8(signal_code(*signal));
            }
            Self::Stop { agent, force } => {
                e.u8(T_STOP);
                e.str(agent);
                e.bool(*force);
            }
            Self::List => e.u8(T_LIST),
            Self::Release { agent } => {
                e.u8(T_RELEASE);
                e.str(agent);
            }
            Self::StartPipe(s) => {
                e.u8(T_START_PIPE);
                e.str(&s.agent);
                e.u32(u32::try_from(s.argv.len()).unwrap_or(u32::MAX));
                for a in &s.argv {
                    e.str(a);
                }
                e.opt_str(&s.cwd);
                e.u32(u32::try_from(s.env.len()).unwrap_or(u32::MAX));
                for (k, v) in &s.env {
                    e.str(k);
                    e.str(v);
                }
            }
            Self::AttachPipes => e.u8(T_ATTACH_PIPES),
            Self::PipeRead { agent } => {
                e.u8(T_PIPE_READ);
                e.str(agent);
            }
            Self::PipeWrite { agent, line } => {
                e.u8(T_PIPE_WRITE);
                e.str(agent);
                e.bytes(line);
            }
        }
        e.0
    }

    pub fn decode(payload: &[u8]) -> Result<Self, LinkError> {
        let mut d = Dec(payload);
        let r = match d.u8()? {
            T_HELLO => Self::Hello { version: d.u16()? },
            T_ATTACH => Self::Attach,
            T_START => {
                let agent = d.str()?;
                let mut argv = Vec::new();
                for _ in 0..d.count()? {
                    argv.push(d.str()?);
                }
                let cwd = d.opt_str()?;
                let mut env = Vec::new();
                for _ in 0..d.count()? {
                    env.push((d.str()?, d.str()?));
                }
                let mut env_remove = Vec::new();
                for _ in 0..d.count()? {
                    env_remove.push(d.str()?);
                }
                Self::Start(StartRequest {
                    agent,
                    argv,
                    cwd,
                    env,
                    env_remove,
                    cols: d.u16()?,
                    rows: d.u16()?,
                    cpu_seconds: d.opt_u64()?,
                    memory_bytes: d.opt_u64()?,
                })
            }
            T_RESIZE => Self::Resize {
                agent: d.str()?,
                cols: d.u16()?,
                rows: d.u16()?,
            },
            T_SIGNAL => Self::Signal {
                agent: d.str()?,
                signal: signal_from(d.u8()?)?,
            },
            T_STOP => Self::Stop {
                agent: d.str()?,
                force: d.bool()?,
            },
            T_LIST => Self::List,
            T_RELEASE => Self::Release { agent: d.str()? },
            T_START_PIPE => {
                let agent = d.str()?;
                let mut argv = Vec::new();
                for _ in 0..d.count()? {
                    argv.push(d.str()?);
                }
                let cwd = d.opt_str()?;
                let mut env = Vec::new();
                for _ in 0..d.count()? {
                    env.push((d.str()?, d.str()?));
                }
                Self::StartPipe(StartPipeRequest {
                    agent,
                    argv,
                    cwd,
                    env,
                })
            }
            T_ATTACH_PIPES => Self::AttachPipes,
            T_PIPE_READ => Self::PipeRead { agent: d.str()? },
            T_PIPE_WRITE => Self::PipeWrite {
                agent: d.str()?,
                line: d.bytes()?.to_vec(),
            },
            _ => return Err(LinkError::Malformed("request (unknown tag)")),
        };
        d.done()?;
        Ok(r)
    }
}

impl Reply {
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Enc::default();
        match self {
            Self::HelloOk {
                version,
                holder_pid,
                holder_start,
            } => {
                e.u8(T_HELLO_OK);
                e.u16(*version);
                e.u32(*holder_pid);
                e.u64(*holder_start);
            }
            Self::Refused {
                code,
                holder_version,
            } => {
                e.u8(T_REFUSED);
                e.u8(*code as u8);
                e.u16(*holder_version);
            }
            Self::Ok => e.u8(T_OK),
            Self::Started { agent, pid } => {
                e.u8(T_STARTED);
                e.str(agent);
                e.u32(*pid);
            }
            Self::Agents(list) => {
                e.u8(T_AGENTS);
                e.u32(u32::try_from(list.len()).unwrap_or(u32::MAX));
                for i in list {
                    e.info(i);
                }
            }
            Self::Terminal(t) => {
                e.u8(T_TERMINAL);
                e.info(&t.info);
                e.bool(t.buffer_dropped);
                e.bytes(&t.buffer);
            }
            Self::AttachDone { count } => {
                e.u8(T_ATTACH_DONE);
                e.u32(*count);
            }
            Self::PipeStarted { agent, pid } => {
                e.u8(T_PIPE_STARTED);
                e.str(agent);
                e.u32(*pid);
            }
            Self::PipeData(p) => {
                e.u8(T_PIPE_DATA);
                e.info(&p.info);
                e.u64(p.dropped);
                e.bool(p.eof);
                e.lines(&p.seen);
                e.lines(&p.fresh);
            }
        }
        e.0
    }

    pub fn decode(payload: &[u8]) -> Result<Self, LinkError> {
        let mut d = Dec(payload);
        let r = match d.u8()? {
            T_HELLO_OK => Self::HelloOk {
                version: d.u16()?,
                holder_pid: d.u32()?,
                holder_start: d.u64()?,
            },
            T_REFUSED => Self::Refused {
                code: Refusal::from_u8(d.u8()?).ok_or(LinkError::Malformed("refusal"))?,
                holder_version: d.u16()?,
            },
            T_OK => Self::Ok,
            T_STARTED => Self::Started {
                agent: d.str()?,
                pid: d.u32()?,
            },
            T_AGENTS => {
                let mut list = Vec::new();
                for _ in 0..d.count()? {
                    list.push(d.info()?);
                }
                Self::Agents(list)
            }
            T_TERMINAL => Self::Terminal(TerminalInfo {
                info: d.info()?,
                buffer_dropped: d.bool()?,
                buffer: d.bytes()?.to_vec(),
            }),
            T_ATTACH_DONE => Self::AttachDone { count: d.u32()? },
            T_PIPE_STARTED => Self::PipeStarted {
                agent: d.str()?,
                pid: d.u32()?,
            },
            T_PIPE_DATA => Self::PipeData(PipeData {
                info: d.info()?,
                dropped: d.u64()?,
                eof: d.bool()?,
                seen: d.lines()?,
                fresh: d.lines()?,
            }),
            _ => return Err(LinkError::Malformed("reply (unknown tag)")),
        };
        d.done()?;
        Ok(r)
    }
}

// ----- frames -----

/// Sends one frame (`payload` is a tag and body), with `terminal` attached to its first bytes when given. It ends within
/// [`FRAME_TIMEOUT`] whatever the stream's write timeout was, and leaves that at [`WRITE_SLICE`].
pub fn write_frame(
    stream: &mut LocalStream,
    payload: &[u8],
    terminal: Option<&TerminalHandle>,
) -> Result<(), LinkError> {
    if payload.is_empty() || payload.len() > MAX_FRAME {
        return Err(LinkError::Malformed("frame (size)"));
    }
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    let began = Instant::now();
    // Each blocked send waits at most a slice, whatever the stream's own write timeout was: with none, or a longer one, a send begun
    // before the frame's last slices could block past its deadline. Not failed for this, as below.
    let _ = stream.set_write_timeout(Some(WRITE_SLICE));
    // In the frame's last slice each send waits only for what is left of its time (and the slice is put back after).
    let mut capped = false;
    let result = send_frame(stream, &frame, terminal, began, &mut capped);
    if capped {
        // A frame that went whole is not failed for this: the peer may have closed right after taking it (macOS then refuses the
        // option, EINVAL). A timeout left short only makes the next frame's sends wake more often; each still waits for its frame's
        // time (`budget`).
        let _ = stream.set_write_timeout(Some(WRITE_SLICE));
    }
    result
}

/// [`write_frame`]'s sends, each started only while the frame has time left and allowed to wait no longer than that time.
fn send_frame(
    stream: &mut LocalStream,
    frame: &[u8],
    terminal: Option<&TerminalHandle>,
    began: Instant,
    capped: &mut bool,
) -> Result<(), LinkError> {
    let mut sent = 0;
    if terminal.is_some() {
        // The terminal goes with the first byte that goes; the rest follows plainly. A send that timed out sent nothing (nor the
        // terminal): it is tried again until the frame's time is up, as a plain send is.
        loop {
            if !budget(stream, began, capped)? {
                continue;
            }
            match stream.send_with_terminal(piece(frame), terminal) {
                Ok(0) => return Err(LinkError::Closed),
                Ok(n) => {
                    sent = n;
                    break;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) if timed_out(&e) => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
    while sent < frame.len() {
        if !budget(stream, began, capped)? {
            continue;
        }
        match stream.write(piece(&frame[sent..])) {
            Ok(0) => return Err(LinkError::Closed),
            Ok(n) => sent += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if timed_out(&e) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Most bytes one send is given, so that each send waits at most once and the frame's deadline holds whatever the socket's buffer.
/// A blocking send waits its write timeout afresh for each kernel buffer it allocates (measured: a whole-frame send ran to 5.4 s
/// against a 5 s limit). On Linux a Unix stream send splits into buffers of at most `SO_SNDBUF / 2 - 64` bytes, and `SO_SNDBUF` is
/// never below `SOCK_MIN_SNDBUF` (2 * (2048 + sizeof(sk_buff)), about 4.5 KiB: buffers of at least about 2.2 KiB), so 2 KiB is one
/// buffer even at the smallest size. A 4 MiB frame, the largest, is 2048 sends.
const SEND_PIECE: usize = 2 << 10;

fn piece(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.len().min(SEND_PIECE)]
}

/// Before a send: a timeout error when the frame's [`FRAME_TIMEOUT`] is spent. In the frame's last two [`WRITE_SLICE`]s the stream
/// is first waited on for writability (`LocalStream::wait_writable`, `poll` on Unix) for at most a slice and what is left: a timed
/// `poll` ends on time, where a socket's own write timeout fires late (a 1 s one by 24-27 ms, measured on Linux, so a send begun with just
/// over a slice left could end past the deadline), and a send of one [`SEND_PIECE`] into room does not wait. In the last slice the stream's
/// write timeout is also cut to what is left after the wait, as a bound where a send waits still (room for less than a piece, or
/// Windows, where the pipe is not polled). Earlier sends are not polled: `poll` reports room only once three quarters of a Unix
/// socket's buffer are free, which halves the rate of a large frame to a reader that keeps up (measured). `Ok(false)`: not writable
/// yet, so no send now (the caller asks again, which ends the frame when its time is spent).
fn budget(stream: &LocalStream, began: Instant, capped: &mut bool) -> Result<bool, LinkError> {
    let left = frame_left(began)?;
    if left >= 2 * WRITE_SLICE {
        return Ok(true);
    }
    if !stream.wait_writable(left.min(WRITE_SLICE))? {
        return Ok(false);
    }
    // What is left after the wait: the send's own bound is that, not what was left before it.
    let left = frame_left(began)?;
    if left < WRITE_SLICE {
        stream.set_write_timeout(Some(left))?;
        *capped = true;
    }
    Ok(true)
}

/// What is left of the [`FRAME_TIMEOUT`] of a frame that began at `began`; a timeout error when nothing is.
fn frame_left(began: Instant) -> Result<Duration, LinkError> {
    let left = FRAME_TIMEOUT.saturating_sub(began.elapsed());
    if left.is_zero() {
        return Err(LinkError::Io(io::Error::from(io::ErrorKind::TimedOut)));
    }
    Ok(left)
}

fn timed_out(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// A frame's payload (tag and body) and the terminal that came with it.
pub type Frame = (Vec<u8>, Option<TerminalHandle>);

/// Reads one frame: its payload and the terminal that came with it. `Ok(None)` when no frame began within `wait` (not zero). A frame
/// begun must end within [`FRAME_TIMEOUT`]: each later receive waits at most what is left of that time and at most a
/// [`WRITE_SLICE`] (and never more than `wait`), and a frame whose last bytes came after it is refused too. The read timeout is set here, so the stream's own is not
/// used. Reads are in exact sizes (the 4-byte length, then the payload), so a terminal sent with one frame is never taken for the
/// next one's.
pub fn read_frame(stream: &mut LocalStream, wait: Duration) -> Result<Option<Frame>, LinkError> {
    read_frame_at(stream, wait, &Instant::now)
}

/// [`read_frame`] with its clock given, so a test can say when the frame's time ran out (#1089).
fn read_frame_at(
    stream: &mut LocalStream,
    wait: Duration,
    now: &dyn Fn() -> Instant,
) -> Result<Option<Frame>, LinkError> {
    let mut terminal: Option<TerminalHandle> = None;
    let mut set = None;
    let mut head = [0u8; 4];
    let mut have = 0;
    let mut began: Option<Instant> = None;
    while have < head.len() {
        receive_budget(stream, wait, began, now, &mut set)?;
        match stream.recv_with_terminal(&mut head[have..]) {
            Ok((0, _)) => return Err(LinkError::Closed),
            Ok((n, t)) => {
                have += n;
                let b = *began.get_or_insert_with(now);
                terminal = terminal.or(t);
                past_deadline(b, now())?;
            }
            // Before the frame: nothing came. Within it: the next receive's budget says whether there is time left.
            Err(e) if timed_out(&e) && began.is_none() => return Ok(None),
            Err(e) if timed_out(&e) => {}
            Err(e) => return Err(e.into()),
        }
    }
    let len = u32::from_be_bytes(head) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(LinkError::Malformed("frame (size)"));
    }
    let began = began.unwrap_or_else(now);
    let mut payload = vec![0u8; len];
    let mut have = 0;
    while have < len {
        receive_budget(stream, wait, Some(began), now, &mut set)?;
        match stream.recv_with_terminal(&mut payload[have..]) {
            Ok((0, _)) => return Err(LinkError::Closed),
            Ok((n, t)) => {
                have += n;
                terminal = terminal.or(t);
                // Also after the receive that completes the frame: its last bytes must have come in time.
                past_deadline(began, now())?;
            }
            Err(e) if timed_out(&e) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(Some((payload, terminal)))
}

/// Before a receive: its read timeout is `wait` until the frame began, then what is left of the frame's time, at most a
/// [`WRITE_SLICE`] as for a send (a long kernel timeout fires late: on Linux one of about 5 s ran up to 0.3 s past it) and at most
/// `wait`; a timeout error once that time is spent. `set` is the timeout last set, so an unchanged one is not set again.
fn receive_budget(
    stream: &LocalStream,
    wait: Duration,
    began: Option<Instant>,
    now: &dyn Fn() -> Instant,
    set: &mut Option<Duration>,
) -> Result<(), LinkError> {
    let timeout = receive_timeout(wait, began, now)?;
    receive_budget_with(timeout, set, |t| stream.set_read_timeout_or_closed(Some(t)))
}

/// [`receive_budget`]'s last step, with the call that sets the stream's read timeout given (so a test can answer as macOS does).
fn receive_budget_with(
    timeout: Duration,
    set: &mut Option<Duration>,
    set_timeout: impl FnOnce(Duration) -> io::Result<ReadTimeout>,
) -> Result<(), LinkError> {
    if *set != Some(timeout) {
        // A peer that has shut the connection (macOS refuses a new timeout then) is not an error here: a receive returns what is
        // buffered or the end at once, so it cannot wait, and the frame's time is still checked. Any other failure is one.
        if set_timeout(timeout)? == ReadTimeout::Set {
            *set = Some(timeout);
        }
    }
    Ok(())
}

/// The read timeout for the next receive (see [`receive_budget`]), or a timeout error when the frame's time is spent.
fn receive_timeout(
    wait: Duration,
    began: Option<Instant>,
    now: &dyn Fn() -> Instant,
) -> Result<Duration, LinkError> {
    let timeout = match began {
        None => wait,
        Some(b) => {
            let left = FRAME_TIMEOUT.saturating_sub(now().saturating_duration_since(b));
            if left.is_zero() {
                return Err(LinkError::Io(io::Error::from(io::ErrorKind::TimedOut)));
            }
            left.min(WRITE_SLICE).min(wait)
        }
    };
    if timeout.is_zero() {
        return Err(LinkError::Io(io::Error::from(io::ErrorKind::TimedOut)));
    }
    Ok(timeout)
}

/// A timeout error when the frame that began at `began` is past its [`FRAME_TIMEOUT`] at `now`.
fn past_deadline(began: Instant, now: Instant) -> Result<(), LinkError> {
    if now.saturating_duration_since(began) > FRAME_TIMEOUT {
        return Err(LinkError::Io(io::Error::from(io::ErrorKind::TimedOut)));
    }
    Ok(())
}

// ----- the node's client -----

/// One terminal `Attach` handed over: what it is, what was buffered, and (for a running agent) the terminal itself.
#[derive(Debug)]
pub struct Held {
    pub info: TerminalInfo,
    pub terminal: Option<TerminalHandle>,
}

/// How long the node waits for the holder's answer to begin.
const REPLY_WAIT: Duration = Duration::from_secs(10);

/// A node's connection to its holder.
pub struct Client {
    stream: LocalStream,
    /// The holder's own process (pid and start time): what the ledger records (decision 9) and the node watches (decision 8).
    pub holder_pid: u32,
    pub holder_start: u64,
    /// The version spoken on this link: [`LINK_VERSION_PIPES`] when the holder keeps pipe agents too, else [`LINK_VERSION`].
    pub version: u16,
}

impl Client {
    /// Connects and says hello with the newest version this build speaks; a holder that answers `VersionMismatch` with an older
    /// version of its own (one built before ADR-0093) is greeted again with that one.
    pub fn connect(socket: &Path) -> Result<Self, LinkError> {
        match Self::connect_with_version(socket, LINK_VERSION_PIPES) {
            Err(LinkError::Refused(Refusal::VersionMismatch, v)) if v == LINK_VERSION => {
                Self::connect_with_version(socket, LINK_VERSION)
            }
            other => other,
        }
    }

    /// As [`Self::connect`] with another version (tests of the mismatch rule). `Refused(VersionMismatch, v)` carries the holder's.
    pub fn connect_with_version(socket: &Path, version: u16) -> Result<Self, LinkError> {
        let mut stream = agora_platform::local_connect(socket)?;
        stream.set_read_timeout(Some(REPLY_WAIT))?;
        stream.set_write_timeout(Some(WRITE_SLICE))?;
        write_frame(&mut stream, &Request::Hello { version }.encode(), None)?;
        match Self::read_reply(&mut stream)?.0 {
            Reply::HelloOk {
                version: spoken,
                holder_pid,
                holder_start,
            } if spoken == version => Ok(Self {
                stream,
                holder_pid,
                holder_start,
                version: spoken,
            }),
            Reply::Refused {
                code,
                holder_version,
            } => Err(LinkError::Refused(code, holder_version)),
            _ => Err(LinkError::Malformed("answer to hello")),
        }
    }

    fn read_reply(stream: &mut LocalStream) -> Result<(Reply, Option<TerminalHandle>), LinkError> {
        // Nothing read within the wait is the holder not answering.
        match read_frame(stream, REPLY_WAIT)? {
            Some((payload, t)) => Ok((Reply::decode(&payload)?, t)),
            None => Err(LinkError::Io(io::Error::from(io::ErrorKind::TimedOut))),
        }
    }

    fn call(&mut self, request: &Request) -> Result<(Reply, Option<TerminalHandle>), LinkError> {
        write_frame(&mut self.stream, &request.encode(), None)?;
        let (reply, terminal) = Self::read_reply(&mut self.stream)?;
        if let Reply::Refused {
            code,
            holder_version,
        } = reply
        {
            return Err(LinkError::Refused(code, holder_version));
        }
        Ok((reply, terminal))
    }

    fn expect_ok(&mut self, request: &Request) -> Result<(), LinkError> {
        match self.call(request)?.0 {
            Reply::Ok => Ok(()),
            _ => Err(LinkError::Malformed("answer")),
        }
    }

    /// Every agent the holder has, running or ended, without taking anything.
    pub fn list(&mut self) -> Result<Vec<AgentInfo>, LinkError> {
        match self.call(&Request::List)?.0 {
            Reply::Agents(a) => Ok(a),
            _ => Err(LinkError::Malformed("answer to list")),
        }
    }

    /// Takes over the terminals (decision 4): the holder ends its own reads first, then hands each terminal and its buffer over.
    pub fn attach(&mut self) -> Result<Vec<Held>, LinkError> {
        write_frame(&mut self.stream, &Request::Attach.encode(), None)?;
        let mut held = Vec::new();
        loop {
            let (reply, terminal) = Self::read_reply(&mut self.stream)?;
            match reply {
                Reply::Terminal(info) => held.push(Held { info, terminal }),
                Reply::AttachDone { count } if count as usize == held.len() => return Ok(held),
                Reply::Refused {
                    code,
                    holder_version,
                } => return Err(LinkError::Refused(code, holder_version)),
                _ => return Err(LinkError::Malformed("answer to attach")),
            }
        }
    }

    /// Starts an agent; returns its pid and its terminal.
    pub fn start(&mut self, request: &StartRequest) -> Result<(u32, TerminalHandle), LinkError> {
        match self.call(&Request::Start(request.clone()))? {
            (Reply::Started { pid, .. }, Some(t)) => Ok((pid, t)),
            _ => Err(LinkError::Malformed("answer to start")),
        }
    }

    pub fn resize(&mut self, agent: &str, cols: u16, rows: u16) -> Result<(), LinkError> {
        self.expect_ok(&Request::Resize {
            agent: agent.into(),
            cols,
            rows,
        })
    }

    pub fn signal(&mut self, agent: &str, signal: HeldSignal) -> Result<(), LinkError> {
        self.expect_ok(&Request::Signal {
            agent: agent.into(),
            signal,
        })
    }

    pub fn stop(&mut self, agent: &str, force: bool) -> Result<(), LinkError> {
        self.expect_ok(&Request::Stop {
            agent: agent.into(),
            force,
        })
    }

    /// ADR-0093: starts a program on pipes; returns its pid.
    pub fn start_pipe(&mut self, request: &StartPipeRequest) -> Result<u32, LinkError> {
        match self.call(&Request::StartPipe(request.clone()))?.0 {
            Reply::PipeStarted { pid, .. } => Ok(pid),
            _ => Err(LinkError::Malformed("answer to start (pipe)")),
        }
    }

    /// ADR-0093: takes over the pipe agents: each with everything the holder kept of its output.
    pub fn attach_pipes(&mut self) -> Result<Vec<HeldPipe>, LinkError> {
        write_frame(&mut self.stream, &Request::AttachPipes.encode(), None)?;
        let mut held = Vec::new();
        loop {
            match Self::read_reply(&mut self.stream)?.0 {
                Reply::PipeData(p) => held.push(p),
                Reply::AttachDone { count } if count as usize == held.len() => return Ok(held),
                Reply::Refused {
                    code,
                    holder_version,
                } => return Err(LinkError::Refused(code, holder_version)),
                _ => return Err(LinkError::Malformed("answer to attach (pipes)")),
            }
        }
    }

    /// ADR-0093: the lines a pipe agent wrote that no node had.
    pub fn pipe_read(&mut self, agent: &str) -> Result<PipeData, LinkError> {
        match self
            .call(&Request::PipeRead {
                agent: agent.into(),
            })?
            .0
        {
            Reply::PipeData(p) => Ok(p),
            _ => Err(LinkError::Malformed("answer to read (pipe)")),
        }
    }

    /// ADR-0093: one line (without its newline) for a pipe agent's input.
    pub fn pipe_write(&mut self, agent: &str, line: &[u8]) -> Result<(), LinkError> {
        self.expect_ok(&Request::PipeWrite {
            agent: agent.into(),
            line: line.to_vec(),
        })
    }

    /// Tells the holder the node has taken an ended agent's status and output. A holder left with nothing ends.
    pub fn release(&mut self, agent: &str) -> Result<(), LinkError> {
        self.expect_ok(&Request::Release {
            agent: agent.into(),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod budget_tests {
    use super::*;

    /// #983: a `List` reply for a holder that keeps `MAX_HELD_AGENTS`, every one at its longest, fits a frame (and decodes); one more
    /// would not, which is why the holder refuses to keep it.
    #[test]
    fn a_full_holder_list_fits_a_frame_and_one_more_agent_would_not() {
        let longest = |n: usize| AgentInfo {
            agent: format!("{n:0>width$}", width = MAX_AGENT_ID),
            pid: u32::MAX,
            state: AgentState::Exited {
                code: Some(i32::MIN),
                signal: Some(i32::MAX),
            },
        };
        assert_eq!(MAX_HELD_AGENTS, 15_251);
        let mut list: Vec<AgentInfo> = (0..MAX_HELD_AGENTS).map(longest).collect();
        let full = Reply::Agents(list.clone()).encode();
        assert_eq!(full.len(), 1 + 4 + MAX_HELD_AGENTS * MAX_LIST_ENTRY);
        assert!(full.len() <= MAX_FRAME);
        assert_eq!(Reply::decode(&full).unwrap(), Reply::Agents(list.clone()));
        list.push(longest(MAX_HELD_AGENTS));
        assert!(Reply::Agents(list).encode().len() > MAX_FRAME);
    }

    #[test]
    fn a_frame_past_its_time_gets_no_receive() {
        let long_ago = Instant::now() - FRAME_TIMEOUT - Duration::from_secs(1);
        let r = receive_timeout(Duration::from_secs(10), Some(long_ago), &Instant::now);
        assert!(matches!(r, Err(LinkError::Io(e)) if e.kind() == io::ErrorKind::TimedOut));
        let r =
            receive_timeout(Duration::from_secs(10), Some(Instant::now()), &Instant::now).unwrap();
        assert!(r <= WRITE_SLICE && !r.is_zero());
        assert_eq!(
            receive_timeout(Duration::from_secs(10), None, &Instant::now).unwrap(),
            Duration::from_secs(10)
        );
    }

    /// #1089 (Codex review of #789): the receive that completes a frame can itself return after the frame's deadline (a kernel
    /// timer that fires late, a thread that was not run in time), and the frame is refused then too. The clock here says the time
    /// ran out from its fourth reading on, the one just after the frame's last receive: the first two readings are taken when the
    /// 4-byte length arrives, the third before the payload's receive. The whole frame is buffered first, so each receive returns
    /// at once and only the clock decides. Without the check after the completing receive, the late read would give the frame.
    #[cfg(unix)]
    #[test]
    fn a_frame_whose_last_receive_ends_after_its_deadline_is_refused() {
        let payload = vec![7u8; 100];
        let read = |late_from: usize| {
            let dir = std::env::temp_dir()
                .join(format!("agora-hl-late-{}-{late_from}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("s.sock");
            let listener = agora_platform::local_listen(&path).unwrap();
            let mut w = agora_platform::local_connect(&path).unwrap();
            let mut r = listener.accept().unwrap();
            let mut frame = (payload.len() as u32).to_be_bytes().to_vec();
            frame.extend_from_slice(&payload);
            io::Write::write_all(&mut w, &frame).unwrap();
            let base = Instant::now();
            let readings = std::cell::Cell::new(0usize);
            let now = || {
                readings.set(readings.get() + 1);
                if readings.get() >= late_from {
                    base + FRAME_TIMEOUT + Duration::from_millis(1)
                } else {
                    base
                }
            };
            let got = read_frame_at(&mut r, Duration::from_secs(10), &now);
            let _ = std::fs::remove_dir_all(&dir);
            (got, readings.get())
        };
        // Out of time only after the last receive: refused.
        let (got, _) = read(4);
        assert!(
            matches!(&got, Err(LinkError::Io(e)) if e.kind() == io::ErrorKind::TimedOut),
            "a frame completed after its deadline was accepted: {got:?}"
        );
        // A clock that never runs out: the frame is read whole, with the four readings counted on above.
        let (got, readings) = read(usize::MAX);
        assert!(
            matches!(&got, Ok(Some((p, None))) if *p == payload),
            "{got:?}"
        );
        assert_eq!(readings, 4, "the clock readings this test counts on");
    }

    #[test]
    fn a_peer_closed_answer_goes_on_without_setting_the_timeout() {
        let mut set = None;
        let r = receive_budget_with(Duration::from_secs(1), &mut set, |_| {
            Ok(ReadTimeout::PeerClosed)
        });
        assert!(r.is_ok());
        assert_eq!(
            set, None,
            "not recorded as set, so the next receive asks again"
        );
    }

    #[test]
    fn any_other_failure_fails_the_receive_and_a_set_one_is_not_asked_twice() {
        let mut set = None;
        let r = receive_budget_with(Duration::from_secs(1), &mut set, |_| {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        });
        assert!(matches!(r, Err(LinkError::Io(e)) if e.kind() == io::ErrorKind::PermissionDenied));
        let mut set = Some(Duration::from_secs(1));
        let r = receive_budget_with(Duration::from_secs(1), &mut set, |_| {
            panic!("an unchanged timeout is not set again")
        });
        assert!(r.is_ok());
    }
}
