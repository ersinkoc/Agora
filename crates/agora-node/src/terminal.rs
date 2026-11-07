//! Terminal actor: one I/O thread per terminal (ADR-0013).
//!
//! Data path: PTY reader thread → bounded channel → this actor → VT feed → [`Snapshot`]s. The actor owns the
//! [`Pty`]; callers send input, resize and kill through a command channel. Before every output chunk the
//! actor drains pending commands, so input outranks output and a flooding terminal only ever delays its own
//! rendering. Terminal bytes never go through the event store (AGENTS.md).

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{
    sync_channel, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError,
};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use agora_platform::{ExitStatus, Pty};
use agora_vt::{AlacrittyTerminal, Cursor, Modes, Size, Terminal};

/// Commands waiting for the actor; bounded so a stuck child pushes back on writers instead of growing memory.
const COMMANDS: usize = 256;
/// How long the actor waits for output before checking commands and the child again.
const TICK: Duration = Duration::from_millis(5);

/// Settings for a terminal actor.
#[derive(Debug, Clone, Copy)]
pub struct TerminalOptions {
    pub size: Size,
    /// Scrollback cap in lines (memory bound only; detection reads the live screen).
    pub scrollback: usize,
}

impl Default for TerminalOptions {
    fn default() -> Self {
        Self {
            size: Size { cols: 80, rows: 24 },
            scrollback: 2000,
        }
    }
}

/// A consistent view of the terminal at one point in the output stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Increases whenever the screen may have changed. Use with [`TerminalHandle::wait_change`].
    pub seq: u64,
    pub size: Size,
    /// Live screen rows, top to bottom, trailing blanks trimmed.
    pub lines: Vec<String>,
    pub cursor: Cursor,
    pub modes: Modes,
    pub title: Option<String>,
    /// `Some` once the process has exited.
    pub exit: Option<ExitStatus>,
}

enum Command {
    Input(Vec<u8>),
    Resize(Size),
    Kill,
}

/// Chunks a subscriber may have queued before it is marked as lagging (16 KiB each at most).
const SUBSCRIBER_QUEUE: usize = 256;

/// One item of a terminal's live output, as seen by a subscriber.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// Raw bytes the process wrote, in order.
    Data(Arc<[u8]>),
    Resize(Size),
    /// The process ended; no more data follows.
    Exited(ExitStatus),
    /// This subscriber fell behind and chunks were dropped (the terminal and other subscribers are not
    /// slowed down). Call [`TerminalHandle::resync`] for a fresh snapshot and continue from there.
    Lagged,
}

struct Subscriber {
    tx: SyncSender<Output>,
    lagged: Arc<AtomicBool>,
}

/// Receiving end of [`TerminalHandle::subscribe`].
pub struct OutputSubscription {
    rx: Receiver<Output>,
    lagged: Arc<AtomicBool>,
}

impl OutputSubscription {
    /// Next item, or `Err(Timeout)`; `Err(Disconnected)` once the terminal is gone and drained.
    /// Reports [`Output::Lagged`] before anything newer when chunks were dropped.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Output, RecvTimeoutError> {
        if self.lagged.load(Ordering::Acquire) {
            return Ok(Output::Lagged);
        }
        let item = self.rx.recv_timeout(timeout)?;
        if self.lagged.load(Ordering::Acquire) {
            return Ok(Output::Lagged);
        }
        Ok(item)
    }
}

struct Shared {
    term: Box<dyn Terminal>,
    seq: u64,
    exit: Option<ExitStatus>,
    subscribers: Vec<Subscriber>,
}

/// Deliver to every subscriber without ever blocking the actor: a full queue marks that subscriber as
/// lagging, a dropped receiver removes it.
fn broadcast(s: &mut Shared, out: &Output) {
    s.subscribers
        .retain(|sub| match sub.tx.try_send(out.clone()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                sub.lagged.store(true, Ordering::Release);
                true
            }
            Err(TrySendError::Disconnected(_)) => false,
        });
}

struct Inner {
    shared: Mutex<Shared>,
    changed: Condvar,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        // A poisoned lock only means another thread panicked; the screen data is still usable.
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn snapshot_of(s: &Shared) -> Snapshot {
    let size = s.term.size();
    Snapshot {
        seq: s.seq,
        size,
        lines: (0..size.rows).map(|r| s.term.line(r)).collect(),
        cursor: s.term.cursor(),
        modes: s.term.modes(),
        title: s.term.title(),
        exit: s.exit,
    }
}

/// Handle to a running terminal actor. Dropping it kills the process and stops the actor.
pub struct TerminalHandle {
    inner: Arc<Inner>,
    commands: SyncSender<Command>,
    pid: u32,
    thread: Option<JoinHandle<()>>,
}

impl TerminalHandle {
    /// Take ownership of a running PTY and start its actor.
    pub fn start(pty: Box<dyn Pty>, options: TerminalOptions) -> io::Result<Self> {
        let pid = pty.pid();
        let term = AlacrittyTerminal::with_scrollback(options.size, options.scrollback);
        let inner = Arc::new(Inner {
            shared: Mutex::new(Shared {
                term: Box::new(term),
                seq: 0,
                exit: None,
                subscribers: Vec::new(),
            }),
            changed: Condvar::new(),
        });
        let (commands, rx) = sync_channel(COMMANDS);
        let actor_inner = Arc::clone(&inner);
        let thread = std::thread::Builder::new()
            .name("agora-terminal".into())
            .spawn(move || run(pty, &actor_inner, &rx))?;
        Ok(Self {
            inner,
            commands,
            pid,
            thread: Some(thread),
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Queue input for the process. Blocks only if 256 commands are already waiting.
    pub fn send_input(&self, bytes: Vec<u8>) -> io::Result<()> {
        self.send(Command::Input(bytes))
    }

    pub fn resize(&self, size: Size) -> io::Result<()> {
        self.send(Command::Resize(size))
    }

    /// Kill the process and its group.
    pub fn kill(&self) -> io::Result<()> {
        self.send(Command::Kill)
    }

    fn send(&self, c: Command) -> io::Result<()> {
        self.commands
            .send(c)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "terminal actor stopped"))
    }

    /// Run `f` against the live terminal (for screen detection) without copying it. Keep `f` short: it holds
    /// the lock the actor needs to feed output.
    pub fn inspect<R>(&self, f: impl FnOnce(&dyn Terminal) -> R) -> R {
        f(self.inner.lock().term.as_ref())
    }

    /// Subscribe to live output. Returns the current screen and a subscription that delivers everything
    /// the process writes after exactly that screen: registration and snapshot happen under one lock, so
    /// there is no gap and no duplicate. A client renders the snapshot, then applies the items in order.
    pub fn subscribe(&self) -> (Snapshot, OutputSubscription) {
        let (tx, rx) = sync_channel(SUBSCRIBER_QUEUE);
        let lagged = Arc::new(AtomicBool::new(false));
        let mut g = self.inner.lock();
        let snap = snapshot_of(&g);
        g.subscribers.push(Subscriber {
            tx,
            lagged: Arc::clone(&lagged),
        });
        (snap, OutputSubscription { rx, lagged })
    }

    /// After [`Output::Lagged`]: discard what is queued, clear the lag, and return a fresh snapshot that
    /// the following items continue from.
    pub fn resync(&self, sub: &OutputSubscription) -> Snapshot {
        let g = self.inner.lock();
        while sub.rx.try_recv().is_ok() {}
        sub.lagged.store(false, Ordering::Release);
        snapshot_of(&g)
    }

    pub fn snapshot(&self) -> Snapshot {
        snapshot_of(&self.inner.lock())
    }

    /// Block until the snapshot sequence is greater than `after` or the timeout passes; returns the
    /// current snapshot either way.
    pub fn wait_change(&self, after: u64, timeout: Duration) -> Snapshot {
        let deadline = Instant::now() + timeout;
        let mut guard = self.inner.lock();
        while guard.seq <= after {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            guard = self
                .inner
                .changed
                .wait_timeout(guard, left)
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
        snapshot_of(&guard)
    }
}

impl Drop for TerminalHandle {
    fn drop(&mut self) {
        let _ = self.commands.try_send(Command::Kill);
        // Closing the command channel makes the actor exit once its drain is done.
        let (dead, _) = sync_channel(1);
        drop(std::mem::replace(&mut self.commands, dead));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn bump(inner: &Inner, f: impl FnOnce(&mut Shared)) {
    let mut g = inner.lock();
    f(&mut g);
    g.seq += 1;
    drop(g);
    inner.changed.notify_all();
}

fn run(mut pty: Box<dyn Pty>, inner: &Inner, commands: &std::sync::mpsc::Receiver<Command>) {
    let mut commands_open = true;
    loop {
        // Input first: drain everything queued before touching output.
        while commands_open {
            match commands.try_recv() {
                Ok(Command::Input(b)) => {
                    let _ = pty.write_input(&b);
                }
                Ok(Command::Resize(size)) => {
                    let _ = pty.resize(size.cols, size.rows);
                    bump(inner, |s| {
                        s.term.resize(size);
                        broadcast(s, &Output::Resize(size));
                    });
                }
                Ok(Command::Kill) => {
                    let _ = pty.kill();
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    commands_open = false;
                    let _ = pty.kill();
                }
            }
        }
        match pty.output().recv_timeout(TICK) {
            Ok(chunk) => bump(inner, |s| {
                s.term.feed(&chunk);
                if !s.subscribers.is_empty() {
                    broadcast(s, &Output::Data(Arc::from(chunk)));
                }
            }),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if !commands_open {
            // The handle is gone: keep draining output until the process is gone, but do not linger.
            if matches!(pty.try_wait(), Ok(Some(_))) {
                break;
            }
        }
    }
    // Output is closed: collect everything the process printed, then record how it ended.
    let status = pty.wait().ok();
    bump(inner, |s| {
        s.exit = status;
        if let Some(st) = status {
            broadcast(s, &Output::Exited(st));
        }
    });
}
