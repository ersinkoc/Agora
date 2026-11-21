//! Terminal actor: one I/O thread per terminal (ADR-0013).
//!
//! Data path: PTY reader thread → bounded channel → this actor → VT feed → [`Snapshot`]s. The actor owns the
//! [`Pty`] and only consumes output, blocking until some arrives (an idle terminal costs no wakeups). Input,
//! resize and kill go straight to the PTY through a [`PtyControl`] on the caller's thread, so input never waits
//! behind output processing and a flooding terminal only ever delays its own rendering. Terminal bytes never
//! go through the event store (AGENTS.md).

use std::fmt::Write as _;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use agora_platform::{ExitStatus, Pty, PtyControl};
use agora_vt::{AlacrittyTerminal, Cursor, Modes, Size, Style, Terminal};

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
    /// The screen with its colours and attributes, as [`Snapshot::to_ansi`] sends it; made only for a client that subscribes
    /// (detection reads `lines` and never pays for it).
    pub repaint: Option<Vec<u8>>,
}

impl Snapshot {
    /// The screen as bytes a terminal emulator can paint (`terminal.raw` resync): every row is positioned
    /// absolutely, so the output does not depend on the client's current cursor or size, and the cursor
    /// ends where the program's cursor is. A subscriber's snapshot carries colours and attributes ([`Snapshot::repaint`]);
    /// any other is plain text.
    pub fn to_ansi(&self) -> Vec<u8> {
        if let Some(r) = &self.repaint {
            return r.clone();
        }
        let mut out = b"\x1b[0m\x1b[2J".to_vec();
        for (i, line) in self.lines.iter().enumerate() {
            out.extend_from_slice(format!("\x1b[{};1H{line}\x1b[K", i + 1).as_bytes());
        }
        out.extend_from_slice(
            format!(
                "\x1b[{};{}H{}",
                self.cursor.row + 1,
                self.cursor.col + 1,
                if self.cursor.visible {
                    "\x1b[?25h"
                } else {
                    "\x1b[?25l"
                }
            )
            .as_bytes(),
        );
        out
    }
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
    waiters: usize,
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

    fn advance(&self, mut shared: MutexGuard<'_, Shared>) {
        shared.seq += 1;
        let has_waiters = shared.waiters != 0;
        drop(shared);
        if has_waiters {
            self.changed.notify_all();
        }
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
        repaint: None,
    }
}

struct PaintRow {
    text: String,
    /// Style changes at byte offsets in `text`.
    styles: Vec<(usize, Style)>,
}

/// The subscriber's screen, captured at registration. Formatting the ANSI repaint only uses this owned data.
struct Repaint {
    snapshot: Snapshot,
    rows: Vec<PaintRow>,
    pen: Style,
}

impl Repaint {
    fn capture(s: &Shared) -> Self {
        let snapshot = snapshot_of(s);
        let size = snapshot.size;
        let rows = (0..size.rows)
            .map(|row| {
                let mut paint = PaintRow {
                    text: String::with_capacity(usize::from(size.cols)),
                    styles: Vec::new(),
                };
                let mut current = Style::default();
                for col in 0..size.cols {
                    let Some(cell) = s.term.cell(row, col) else {
                        break;
                    };
                    if cell.spacer {
                        continue;
                    }
                    if cell.style != current {
                        paint.styles.push((paint.text.len(), cell.style));
                        current = cell.style;
                    }
                    paint.text.push(cell.ch);
                }
                paint
            })
            .collect();
        Self {
            snapshot,
            rows,
            pen: s.term.pen(),
        }
    }

    fn into_snapshot(mut self) -> Snapshot {
        let mut out = String::from("\x1b[0m\x1b[2J");
        for (row, paint) in self.rows.into_iter().enumerate() {
            let _ = write!(out, "\x1b[{};1H", row + 1);
            let mut start = 0;
            for (offset, style) in paint.styles {
                out.push_str(&paint.text[start..offset]);
                style.append_sgr(&mut out);
                start = offset;
            }
            out.push_str(&paint.text[start..]);
            out.push_str("\x1b[0m\x1b[K");
        }
        let c = self.snapshot.cursor;
        let _ = write!(
            out,
            "\x1b[{};{}H{}",
            c.row + 1,
            c.col + 1,
            if c.visible { "\x1b[?25h" } else { "\x1b[?25l" }
        );
        // Live output continues from the captured cursor, input modes and pen.
        let m = self.snapshot.modes;
        out.push_str(if m.bracketed_paste {
            "\x1b[?2004h"
        } else {
            "\x1b[?2004l"
        });
        out.push_str(if m.application_cursor {
            "\x1b[?1h"
        } else {
            "\x1b[?1l"
        });
        self.pen.append_sgr(&mut out);
        self.snapshot.repaint = Some(out.into_bytes());
        self.snapshot
    }
}

/// Handle to a running terminal actor. Dropping it kills the process and stops the actor.
pub struct TerminalHandle {
    inner: Arc<Inner>,
    control: Option<PtyControl>,
    pid: u32,
    thread: Option<JoinHandle<()>>,
    /// ADR-0079: the process belongs to the node's terminal holder (it outlives the node).
    held: bool,
}

impl TerminalHandle {
    /// Take ownership of a running PTY and start its actor.
    pub fn start(pty: Box<dyn Pty>, options: TerminalOptions) -> io::Result<Self> {
        Self::start_with_output(pty, options, &[])
    }

    /// [`Self::start`] for a terminal the node takes over (ADR-0079 decision 4): `earlier` is what the terminal showed since the node
    /// last read it (the holder's buffer), fed to the screen before anything live.
    pub fn start_with_output(
        pty: Box<dyn Pty>,
        options: TerminalOptions,
        earlier: &[u8],
    ) -> io::Result<Self> {
        let pid = pty.pid();
        let control = pty.control()?;
        let mut term = AlacrittyTerminal::with_scrollback(options.size, options.scrollback);
        if !earlier.is_empty() {
            term.feed(earlier);
        }
        let inner = Arc::new(Inner {
            shared: Mutex::new(Shared {
                term: Box::new(term),
                seq: 0,
                exit: None,
                subscribers: Vec::new(),
                waiters: 0,
            }),
            changed: Condvar::new(),
        });
        let actor_inner = Arc::clone(&inner);
        let thread = std::thread::Builder::new()
            .name("agora-terminal".into())
            .spawn(move || run(pty, &actor_inner))?;
        Ok(Self {
            inner,
            control: Some(control),
            pid,
            thread: Some(thread),
            held: false,
        })
    }

    /// Marks this terminal's process as the holder's (see [`Self::is_held`]).
    #[must_use]
    pub fn into_held(mut self) -> Self {
        self.held = true;
        self
    }

    /// ADR-0079: the process belongs to the terminal holder and outlives the node.
    pub fn is_held(&self) -> bool {
        self.held
    }

    /// ADR-0079: the node is stopping and leaves a held agent running: reading its terminal ends, and dropping this handle no longer
    /// ends it. A terminal the node owns outright is unaffected (dropping it kills the agent, as before).
    pub fn detach(&self) {
        if let Some(c) = &self.control {
            c.detach();
        }
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    fn control(&self) -> io::Result<&PtyControl> {
        self.control
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "terminal stopped"))
    }

    /// A handle that writes input without the terminal (or whatever lock reached it): clone it, let go of the locks, then
    /// write. A program that never reads its input blocks the writer, and only the writer.
    pub fn input_handle(&self) -> io::Result<PtyControl> {
        self.control().cloned()
    }

    /// Send input to the process, immediately and on the caller's thread. Blocks only while the process is
    /// not reading and the terminal's input queue is full.
    pub fn send_input(&self, bytes: Vec<u8>) -> io::Result<()> {
        self.control()?.write_input(&bytes)
    }

    /// Resize the terminal. The PTY and the screen change together, between two output chunks.
    pub fn resize(&self, size: Size) -> io::Result<()> {
        let control = self.control()?;
        let mut g = self.inner.lock();
        // The size the screen will really have (it keeps a minimum width): the process's terminal and the clients get that one, so a
        // request for one column leaves the screen, the process and the broadcast in agreement, not the process at one column.
        let size = g.term.clamp_size(size);
        // The same size again changes nothing; a console that is resized needlessly can drop input typed at that moment (Git `sh`
        // on Windows lost the first key this way).
        if g.term.size() == size {
            return Ok(());
        }
        control.resize(size.cols, size.rows)?;
        g.term.resize(size);
        broadcast(&mut g, &Output::Resize(size));
        self.inner.advance(g);
        Ok(())
    }

    /// Kill the process and its group.
    pub fn kill(&self) -> io::Result<()> {
        self.control()?.kill()
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
        let repaint = {
            let mut g = self.inner.lock();
            let repaint = Repaint::capture(&g);
            g.subscribers.push(Subscriber {
                tx,
                lagged: Arc::clone(&lagged),
            });
            repaint
        };
        (repaint.into_snapshot(), OutputSubscription { rx, lagged })
    }

    /// After [`Output::Lagged`]: discard what is queued, clear the lag, and return a fresh snapshot that
    /// the following items continue from.
    pub fn resync(&self, sub: &OutputSubscription) -> Snapshot {
        let repaint = {
            let g = self.inner.lock();
            while sub.rx.try_recv().is_ok() {}
            sub.lagged.store(false, Ordering::Release);
            Repaint::capture(&g)
        };
        repaint.into_snapshot()
    }

    /// How the process ended, once it has. Cheap: no screen is copied (the supervisor polls this).
    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.inner.lock().exit
    }

    pub fn snapshot(&self) -> Snapshot {
        snapshot_of(&self.inner.lock())
    }

    /// Whether the program turned on bracketed paste (DECSET 2004), read without copying the screen.
    pub fn bracketed_paste(&self) -> bool {
        self.inner.lock().term.modes().bracketed_paste
    }

    /// Block until the snapshot sequence is greater than `after` or the timeout passes; returns the
    /// current snapshot either way.
    pub fn wait_change(&self, after: u64, timeout: Duration) -> Snapshot {
        let deadline = Instant::now() + timeout;
        let mut guard = self.inner.lock();
        if guard.seq <= after {
            guard.waiters += 1;
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
            guard.waiters -= 1;
        }
        snapshot_of(&guard)
    }
}

impl Drop for TerminalHandle {
    fn drop(&mut self) {
        // Kill first, then release our copy of the PTY before waiting for the actor: a child blocked
        // writing into a full terminal can only finish dying once every copy of the master is closed.
        if let Some(c) = self.control.take() {
            let _ = c.kill();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn bump(inner: &Inner, f: impl FnOnce(&mut Shared)) {
    let mut g = inner.lock();
    f(&mut g);
    inner.advance(g);
}

/// How often an idle actor checks that the child still exists. Normally the output channel closing ends the
/// loop at once; this only matters when a descendant that escaped the process group keeps the PTY open.
const IDLE_CHECK: Duration = Duration::from_secs(1);

fn run(mut pty: Box<dyn Pty>, inner: &Inner) {
    loop {
        match pty.output().recv_timeout(IDLE_CHECK) {
            Ok(chunk) => bump(inner, |s| {
                s.term.feed(&chunk);
                if !s.subscribers.is_empty() {
                    broadcast(s, &Output::Data(Arc::from(chunk)));
                }
            }),
            // Closed: the process side of the terminal is gone (killed or exited).
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if matches!(pty.try_wait(), Ok(Some(_))) {
                    break;
                }
            }
        }
    }
    // Collect everything the process printed, then record how it ended.
    let status = pty.wait().ok();
    bump(inner, |s| {
        s.exit = status;
        if let Some(st) = status {
            broadcast(s, &Output::Exited(st));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inner(term: AlacrittyTerminal) -> Arc<Inner> {
        Arc::new(Inner {
            shared: Mutex::new(Shared {
                term: Box::new(term),
                seq: 0,
                exit: None,
                subscribers: Vec::new(),
                waiters: 0,
            }),
            changed: Condvar::new(),
        })
    }

    #[test]
    fn a_repaint_uses_the_capture_after_the_live_screen_changes() {
        let size = Size { cols: 16, rows: 4 };
        let mut live = AlacrittyTerminal::new(size);
        live.feed("\x1b[?2004h\x1b[?1h\x1b[31m界\x1b[32mX".as_bytes());
        let pen = live.pen();
        let original = inner(live);
        let captured = Repaint::capture(&original.lock());

        bump(&original, |s| {
            s.term.feed(b"\x1b[0m\x1b[2J\x1b[?2004l\x1b[?1l")
        });
        let snap = captured.into_snapshot();
        let mut mirror = AlacrittyTerminal::new(size);
        mirror.feed(&snap.to_ansi());
        assert_eq!(mirror.line(0), snap.lines[0]);
        assert_eq!(mirror.line(0), "界X");
        assert_eq!(mirror.pen(), pen);
        assert_eq!(mirror.cursor(), snap.cursor);
        assert_eq!(mirror.modes(), snap.modes);
        assert_ne!(mirror.modes(), original.lock().term.modes());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn an_update_wakes_every_registered_snapshot_waiter() {
        let handle = TerminalHandle {
            inner: inner(AlacrittyTerminal::new(Size { cols: 16, rows: 4 })),
            control: None,
            pid: 0,
            thread: None,
            held: false,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            for _ in 0..2 {
                let (tx, handle) = (tx.clone(), &handle);
                scope.spawn(move || {
                    tx.send(handle.wait_change(0, Duration::from_secs(10)).seq)
                        .unwrap();
                });
            }
            let until = Instant::now() + Duration::from_secs(3);
            while handle.inner.lock().waiters < 2 {
                assert!(Instant::now() < until);
                std::thread::yield_now();
            }
            bump(&handle.inner, |_| {});
            for _ in 0..2 {
                assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), 1);
            }
        });
        assert_eq!(handle.inner.lock().waiters, 0);
    }
}
