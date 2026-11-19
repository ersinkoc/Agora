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

use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use agora_platform::{ExitStatus, HeldControl, HeldSignal, ProcessWatch, TerminalHandle};

pub use crate::holder_link::StartRequest;
use crate::holder_link::{socket_path, AgentInfo, AgentState, Client, Held, LinkError, Refusal};

/// How long a start waits for a holder to listen, and a connection for an answer other than `Busy`.
const START_WAIT: Duration = Duration::from_secs(5);
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
    /// The holder's process, as the ledger records it (ADR-0079 decision 9).
    pub pid: u32,
    pub start: u64,
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
        self.call(|c| c.start(request))
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

fn open(setup: &HolderSetup) -> Result<(Arc<HolderSession>, Vec<Held>), Why> {
    let socket = socket_path(&setup.home);
    let end = Instant::now() + START_WAIT;
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
    let session = Arc::new(HolderSession {
        client: Mutex::new(client),
        watch,
        broken: AtomicBool::new(false),
        pid,
        start,
    });
    Ok((session, held))
}

/// Starts `agora hold` detached, in a session of its own: never bound to its starter (a holder that died with the node would defeat
/// it). `Err` when it cannot be started that way (`setsid` refused), which is "no holder here".
fn start_holder(setup: &HolderSetup) -> io::Result<u32> {
    let run = setup.home.join("run");
    agora_platform::ensure_private_dir(&run)?;
    let mut cmd = std::process::Command::new(&setup.exe);
    cmd.arg("hold").arg("--home").arg(&setup.home);
    Ok(agora_platform::spawn_detached(&mut cmd, &run.join("hold.log"))?.pid)
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
}

impl HolderHub {
    pub fn new(setup: HolderSetup) -> Self {
        Self {
            setup,
            state: Mutex::new(HubState::default()),
        }
    }

    fn st(&self) -> MutexGuard<'_, HubState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// At the node's start: attaches to the holder that is already running (what a node before this one left), with the terminals and
    /// output it holds. `None` when no holder runs or it cannot be used (then agents run in the node).
    pub fn attach_existing(&self) -> Option<(Arc<HolderSession>, Vec<Held>)> {
        if !agora_platform::HOLDER_BUILT {
            return None;
        }
        let mut st = self.st();
        match open(&self.setup) {
            Ok((session, held)) => {
                st.session = Some(Arc::clone(&session));
                Some((session, held))
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
        if st.unavailable_until.is_some_and(|t| Instant::now() < t) {
            return None;
        }
        let session = match open(&self.setup) {
            Ok((s, held)) if held.is_empty() => Some(s),
            // A holder that still holds agents this node never attached (its own link broke while the holder lived) is not used: its
            // agents' terminals cannot be taken up here, and starting more in it would hide them. They stay as the holder keeps them.
            Ok(_) | Err(Why::Other) => None,
            Err(Why::NoHolder) => self.start_and_open(),
        };
        match session {
            Some(s) => {
                st.unavailable_until = None;
                st.session = Some(Arc::clone(&s));
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
            match open(&self.setup) {
                Ok((s, held)) if held.is_empty() => return Some(s),
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
        self.st()
            .session
            .as_ref()
            .filter(|s| !s.gone())
            .map(|s| (s.pid, s.start))
    }
}
