//! Terminal actor: one I/O thread per terminal (ADR-0013).
//!
//! Data path: PTY reader thread → bounded channel → this actor → VT feed → [`Snapshot`]s. The actor owns the
//! [`Pty`] and only consumes output, blocking until some arrives (an idle terminal costs no wakeups). Input,
//! resize and kill go straight to the PTY through a [`PtyControl`] on the caller's thread, so input never waits
//! behind output processing and a flooding terminal only ever delays its own rendering. Terminal bytes never
//! go through the event store (AGENTS.md).

use std::fmt::Write as _;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use agora_platform::{ExitStatus, Pty, PtyControl};
use agora_vt::{
    AlacrittyTerminal, Cursor, CursorShape, Modes, MouseEncoding, MouseTracking, Size, Style,
    Terminal,
};

use crate::history::HistoryCapture;

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
    snapshot_from(s.term.as_ref(), s.seq, s.exit)
}

fn snapshot_from(term: &dyn Terminal, seq: u64, exit: Option<ExitStatus>) -> Snapshot {
    let size = term.size();
    Snapshot {
        seq,
        size,
        lines: (0..size.rows).map(|r| term.line(r)).collect(),
        cursor: term.cursor(),
        modes: term.modes(),
        title: term.title(),
        exit,
        repaint: None,
    }
}

/// The repaint a subscriber of `term` gets (the kind 1 frame, ADR-0100): the same bytes [`TerminalHandle::subscribe`] sends, for
/// clients' tests and tools that hold an emulator rather than a running terminal.
pub fn repaint_of(term: &dyn Terminal) -> Vec<u8> {
    Repaint::capture(term, 0, None)
        .into_snapshot()
        .repaint
        .unwrap_or_default()
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

/// `CSI ? n h` or `CSI ? n l`.
fn private_mode(out: &mut String, n: u16, on: bool) {
    let _ = write!(out, "\x1b[?{n}{}", if on { 'h' } else { 'l' });
}

/// Every mode the node's emulator tracks, as an explicit set or reset (ADR-0100 decision 1, step 4), so a window that kept modes
/// from before (a reconnect) ends with the program's. Modes the emulator does not track (mouse 9, the urxvt encoding 1015,
/// modifyOtherKeys, 47/1047, the scroll region) are left alone: resetting what the node cannot see could switch off what the
/// program switched on while the window watched. Origin mode (DECOM) is not restored either: cursor positions under it are relative to
/// the scroll region, which is not tracked, so restoring it would put the cursor, and every later position the program sends, on
/// the wrong row (Codex review of ADR-0100); it waits until the margins can be restored. Two known gaps: xterm.js (the web window)
/// switches mouse mode 9 off on any of the 1000/1002/1003 resets, so there the repaint does end a 9 the window had; and only the
/// kitty keyboard protocol's current flags are carried, not the program's stack, so a pop after a late attach gives the window none.
fn append_modes(out: &mut String, m: &Modes) {
    for n in [1000, 1002, 1003] {
        private_mode(out, n, false);
    }
    match m.mouse_tracking {
        MouseTracking::Off => {}
        MouseTracking::Click => private_mode(out, 1000, true),
        MouseTracking::Drag => private_mode(out, 1002, true),
        MouseTracking::Motion => private_mode(out, 1003, true),
    }
    for n in [1005, 1006] {
        private_mode(out, n, false);
    }
    match m.mouse_encoding {
        MouseEncoding::Default => {}
        MouseEncoding::Utf8 => private_mode(out, 1005, true),
        MouseEncoding::Sgr => private_mode(out, 1006, true),
    }
    private_mode(out, 1004, m.focus_reporting);
    private_mode(out, 1007, m.alternate_scroll);
    private_mode(out, 2004, m.bracketed_paste);
    private_mode(out, 1, m.application_cursor);
    // DECKPAM or DECKPNM.
    out.push_str(if m.application_keypad {
        "\x1b="
    } else {
        "\x1b>"
    });
    private_mode(out, 7, m.autowrap);
    out.push_str(if m.insert { "\x1b[4h" } else { "\x1b[4l" });
    out.push_str(if m.newline { "\x1b[20h" } else { "\x1b[20l" });
    // The kitty keyboard flags: pop whatever stack the window holds, then set the program's flags without a stack entry.
    let _ = write!(out, "\x1b[<32u\x1b[={};1u", m.keyboard_flags);
}

/// The cursor (ADR-0100 decision 1, step 5): position, visibility and shape. The default shape is `CSI 0 SP q`, which gives the
/// window its user's own cursor back. Mode 12 (blink alone) is never written: xterm.js applies it to the window's own cursor-blink
/// setting, which would outlive the program, so a blink without a shape the program chose is not carried (a known gap); a chosen
/// shape carries its blink in DECSCUSR.
fn append_cursor(out: &mut String, c: &Cursor) {
    let _ = write!(
        out,
        "\x1b[{};{}H{}",
        c.row + 1,
        c.col + 1,
        if c.visible { "\x1b[?25h" } else { "\x1b[?25l" }
    );
    let shape = match c.shape {
        CursorShape::Default => 0,
        CursorShape::Block => 2,
        CursorShape::Underline => 4,
        CursorShape::Bar => 6,
    };
    if shape == 0 {
        out.push_str("\x1b[0 q");
    } else {
        // DECSCUSR: the odd value of each pair blinks.
        let _ = write!(out, "\x1b[{} q", shape - u8::from(c.blink));
    }
}

impl Repaint {
    fn capture(term: &dyn Terminal, seq: u64, exit: Option<ExitStatus>) -> Self {
        let snapshot = snapshot_from(term, seq, exit);
        let size = snapshot.size;
        let rows = (0..size.rows)
            .map(|row| {
                let mut paint = PaintRow {
                    text: String::with_capacity(usize::from(size.cols)),
                    styles: Vec::new(),
                };
                let mut current = Style::default();
                for col in 0..size.cols {
                    let Some(cell) = term.cell(row, col) else {
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
            pen: term.pen(),
        }
    }

    /// The kind 1 repaint (ADR-0100 decision 1), in this order: the program's screen (normal or alternate), the state the drawing
    /// depends on, the drawing, the modes, the cursor, the pen. Every byte is made here from the captured state; nothing is copied
    /// from the program's output.
    fn into_snapshot(mut self) -> Snapshot {
        let m = self.snapshot.modes;
        // 1. The program's screen first, so the drawing lands on it.
        let mut out = String::from(if m.alt_screen {
            "\x1b[?1049h"
        } else {
            "\x1b[?1049l"
        });
        // 2. Rows land at absolute positions (origin off), characters overwrite (insert off), from a plain pen, in ASCII (G0 is ASCII and
        // shifted in: a window left in the line-drawing set by an earlier program would draw the whole screen in it). The node does
        // not track the character sets, so this is a protective reset, not the program's state.
        out.push_str("\x1b[?6l\x1b[4l\x1b[0m\x1b(B\x0f");
        // 3. The screen, every row at an absolute position with its styles.
        out.push_str("\x1b[2J");
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
        // 4-6. Live output continues from the captured modes, cursor and pen.
        append_modes(&mut out, &m);
        append_cursor(&mut out, &self.snapshot.cursor);
        self.pen.append_sgr(&mut out);
        self.snapshot.repaint = Some(out.into_bytes());
        self.snapshot
    }
}

/// The next [`TerminalHandle::launch`]; counts up for as long as the node process runs.
static LAUNCHES: AtomicU64 = AtomicU64::new(1);

/// Handle to a running terminal actor. Dropping it kills the process and stops the actor.
pub struct TerminalHandle {
    inner: Arc<Inner>,
    control: Option<PtyControl>,
    pid: u32,
    launch: u64,
    /// No file this process wrote is older than this (see [`Self::started`]).
    started: Option<SystemTime>,
    thread: Option<JoinHandle<()>>,
    /// ADR-0079: the process belongs to the node's terminal holder (it outlives the node).
    held: bool,
    /// #933: the generated Git configuration of this launch (ADR-0098), removed once the process is ended (fields drop after
    /// [`Drop::drop`] killed it). Only a confined launch has one, and a confined agent is never the holder's.
    launch_file: Option<crate::gitconfig::LaunchFile>,
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
            launch: LAUNCHES.fetch_add(1, Ordering::Relaxed),
            started: None,
            thread: Some(thread),
            held: false,
            launch_file: None,
        })
    }

    /// #933: keeps this launch's generated Git file for as long as the handle, so it is removed when the process ends or is stopped.
    #[must_use]
    pub(crate) fn with_launch_file(mut self, file: Option<crate::gitconfig::LaunchFile>) -> Self {
        self.launch_file = file;
        self
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

    /// Which process this handle runs, as a number no other handle of this node process has (a pid can be used again): what the node
    /// learns about one process of an agent (its session file, #925) is kept under it, so it is never taken for the agent's next one.
    pub fn launch(&self) -> u64 {
        self.launch
    }

    /// A time before this process started (taken before it was spawned, or for one taken up from the holder, a bound the ledger kept):
    /// no file it wrote is older (#925). `None` when the node does not know one.
    pub fn started(&self) -> Option<SystemTime> {
        self.started
    }

    /// Records [`Self::started`].
    #[must_use]
    pub fn started_after(mut self, bound: SystemTime) -> Self {
        self.started = Some(bound);
        self
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
        let (_, snapshot, sub) = self.subscribe_with_history(0);
        (snapshot, sub)
    }

    /// [`Self::subscribe`] for a window that asks for `lines` lines of history (ADR-0100 decision 2); see
    /// [`TerminalWatch::subscribe_with_history`].
    pub fn subscribe_with_history(
        &self,
        lines: usize,
    ) -> (Vec<Vec<u8>>, Snapshot, OutputSubscription) {
        self.watch().subscribe_with_history(lines)
    }

    /// A handle to subscribe with after letting go of whatever lock reached this terminal (the supervisor's), so copying and
    /// formatting a history (ADR-0100) never holds up other commands, input among them. It does not keep the process running.
    pub fn watch(&self) -> TerminalWatch {
        TerminalWatch {
            inner: Arc::clone(&self.inner),
        }
    }

    /// After [`Output::Lagged`]: discard what is queued, clear the lag, and return a fresh snapshot that
    /// the following items continue from. Never with history (ADR-0100 decision 2): a window that wants it subscribes again.
    pub fn resync(&self, sub: &OutputSubscription) -> Snapshot {
        let repaint = {
            let g = self.inner.lock();
            while sub.rx.try_recv().is_ok() {}
            sub.lagged.store(false, Ordering::Release);
            Repaint::capture(g.term.as_ref(), g.seq, g.exit)
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

/// A terminal to subscribe to, held without its [`TerminalHandle`] (see [`TerminalHandle::watch`]).
pub struct TerminalWatch {
    inner: Arc<Inner>,
}

impl TerminalWatch {
    /// [`TerminalHandle::subscribe`] that also returns the kind 4 frames putting the newest `lines` scrollback lines into the
    /// window's own scrollback (ADR-0100 decision 2), sent before the snapshot: none for 0, while the program is on the alternate
    /// screen, or when nothing is kept. History, screen and registration are captured under one lock, so the history ends where the
    /// snapshot begins; the cells are copied under it (bounded by the byte cap) and formatted after it is released.
    pub fn subscribe_with_history(
        &self,
        lines: usize,
    ) -> (Vec<Vec<u8>>, Snapshot, OutputSubscription) {
        let (tx, rx) = sync_channel(SUBSCRIBER_QUEUE);
        let lagged = Arc::new(AtomicBool::new(false));
        let (history, repaint) = {
            let mut g = self.inner.lock();
            let history = (lines > 0).then(|| HistoryCapture::capture(g.term.as_ref(), lines));
            let repaint = Repaint::capture(g.term.as_ref(), g.seq, g.exit);
            // F1481: the process already ended, so its `Exited` went out before this subscriber was there. It gets it first (an
            // empty queue has room), and the window hears the code rather than an unknown end once the terminal is let go.
            if let Some(st) = g.exit {
                let _ = tx.try_send(Output::Exited(st));
            }
            g.subscribers.push(Subscriber {
                tx,
                lagged: Arc::clone(&lagged),
            });
            (history, repaint)
        };
        let frames = history.map(HistoryCapture::into_frames).unwrap_or_default();
        (
            frames,
            repaint.into_snapshot(),
            OutputSubscription { rx, lagged },
        )
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

/// How often the actor checks that the child still exists, idle or not (F1480). Normally the output channel closing ends the
/// loop at once; this only matters when a descendant that escaped the process group keeps the PTY open.
const IDLE_CHECK: Duration = Duration::from_secs(1);

fn run(mut pty: Box<dyn Pty>, inner: &Inner) {
    // F1480: a descendant outside the process group that keeps the terminal open and writes more often than `IDLE_CHECK` never lets
    // the channel go quiet, so the child is also looked for at that pace while output flows (one clock read per chunk). Once it is
    // gone, what it printed last still has one more `IDLE_CHECK` to reach the screen before the loop ends.
    let mut next_check = Instant::now() + IDLE_CHECK;
    let mut ended = false;
    loop {
        match pty.output().recv_timeout(IDLE_CHECK) {
            Ok(chunk) => {
                bump(inner, |s| {
                    s.term.feed(&chunk);
                    if !s.subscribers.is_empty() {
                        broadcast(s, &Output::Data(Arc::from(chunk)));
                    }
                });
                if Instant::now() >= next_check {
                    if ended {
                        break;
                    }
                    ended = matches!(pty.try_wait(), Ok(Some(_)));
                    next_check = Instant::now() + IDLE_CHECK;
                }
            }
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
        let captured = {
            let g = original.lock();
            Repaint::capture(g.term.as_ref(), g.seq, g.exit)
        };

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

    /// The repaint of a terminal fed `bytes`, and that terminal.
    fn source(size: Size, bytes: &[u8]) -> (AlacrittyTerminal, String) {
        let mut t = AlacrittyTerminal::new(size);
        t.feed(bytes);
        let r = String::from_utf8_lossy(&repaint_of(&t)).into_owned();
        (t, r)
    }

    /// Where `needle` is in `hay`; fails the test when it is missing.
    #[allow(clippy::unwrap_used)]
    fn at(hay: &str, needle: &str) -> usize {
        hay.find(needle)
            .ok_or_else(|| format!("{needle:?} missing from {hay:?}"))
            .unwrap()
    }

    /// Every switch the program can make, all on (and an alternate screen with its own text).
    const ALL_ON: &[u8] = b"main\x1b[?1049h\x1b[HALT\x1b[?1002h\x1b[?1006h\x1b[?1004h\x1b[?2004h\x1b[?1h\x1b=\x1b[4h\x1b[20h\x1b[>5u\x1b[5 q\x1b[2;3H";
    /// The opposite of every power-on mode (autowrap and alternate scroll are on at power-on).
    const ALL_OFF: &[u8] = b"\x1b[?7l\x1b[?1007l\x1b[?25l";

    /// ADR-0100 decision 1: every mode the emulator tracks is written as an explicit set (all on) or reset (all off).
    #[test]
    fn every_tracked_mode_is_set_or_reset_explicitly() {
        let size = Size { cols: 16, rows: 4 };
        let (_, on) = source(size, ALL_ON);
        for seq in [
            "\x1b[?1049h",
            "\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1002h",
            "\x1b[?1005l\x1b[?1006l\x1b[?1006h",
            "\x1b[?1004h",
            "\x1b[?1007h",
            "\x1b[?2004h",
            "\x1b[?1h",
            "\x1b=",
            "\x1b[?7h",
            "\x1b[4h",
            "\x1b[20h",
            "\x1b[<32u\x1b[=5;1u",
            "\x1b[2;3H\x1b[?25h\x1b[5 q",
        ] {
            at(&on, seq);
        }
        let (_, off) = source(size, ALL_OFF);
        for seq in [
            "\x1b[?1049l",
            "\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1004l",
            "\x1b[?1007l",
            "\x1b[?2004l",
            "\x1b[?1l",
            "\x1b>",
            "\x1b[?7l",
            "\x1b[20l",
            "\x1b[<32u\x1b[=0;1u",
            "\x1b[1;1H\x1b[?25l\x1b[0 q",
        ] {
            at(&off, seq);
        }
        assert!(!off.contains("\x1b[?1000h") && !off.contains("\x1b[?12"));
        // Each mouse mode and encoding on its own.
        for (prog, set) in [
            ("\x1b[?1000h", "\x1b[?1003l\x1b[?1000h"),
            ("\x1b[?1003h", "\x1b[?1003l\x1b[?1003h"),
            ("\x1b[?1005h", "\x1b[?1006l\x1b[?1005h"),
            ("\x1b[4 q", "\x1b[4 q"),
            ("\x1b[1 q", "\x1b[1 q"),
            ("\x1b[6 q", "\x1b[6 q"),
            // A blink with no shape chosen keeps the window's own shape, and mode 12 is never written (see `append_cursor`).
            ("\x1b[?12h", "\x1b[0 q\x1b[0m"),
            ("\x1b[3 q\x1b[0 q", "\x1b[0 q\x1b[0m"),
        ] {
            at(&source(size, prog.as_bytes()).1, set);
        }
    }

    /// The order of ADR-0100 decision 1: screen, drawing state, drawing, modes, cursor, pen. On the alternate screen the drawing comes
    /// after entering it, so it lands there and the window's main screen is not overwritten.
    #[test]
    fn the_repaint_enters_the_screen_then_draws_then_sets_modes_cursor_and_pen() {
        let size = Size { cols: 16, rows: 4 };
        let (_, r) = source(size, ALL_ON);
        let order = [
            at(&r, "\x1b[?1049h"),
            at(&r, "\x1b[?6l\x1b[4l\x1b[0m\x1b(B\x0f"),
            at(&r, "\x1b[2J"),
            at(&r, "\x1b[1;1HALT"),
            at(&r, "\x1b[?1000l"),
            at(&r, "\x1b[=5;1u"),
            at(&r, "\x1b[2;3H\x1b[?25h"),
            at(&r, "\x1b[5 q"),
        ];
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?} in {r:?}");
        assert!(r.ends_with("\x1b[0m"), "the pen is last: {r:?}");
        // Fed to a window on its main screen, the drawing is on the alternate screen; leaving it shows the window's own main screen.
        let mut window = AlacrittyTerminal::new(size);
        window.feed(b"window's own");
        window.feed(r.as_bytes());
        assert!(window.modes().alt_screen);
        assert_eq!(window.line(0), "ALT");
        window.feed(b"\x1b[?1049l");
        assert_eq!(window.line(0), "window's own");
    }

    /// A repaint fed to a fresh window, and to one that kept every mode from before (a reconnect: D3), gives the source's modes,
    /// cursor and screen.
    #[test]
    fn a_repaint_reproduces_the_modes_on_a_fresh_and_on_a_stale_window() {
        let size = Size { cols: 16, rows: 4 };
        for bytes in [ALL_ON, ALL_OFF, b"" as &[u8], b"x\x1b[?1000h\x1b[?1015h"] {
            let (src, r) = source(size, bytes);
            for stale in [b"" as &[u8], ALL_ON, ALL_OFF] {
                let mut window = AlacrittyTerminal::new(size);
                window.feed(stale);
                window.feed(r.as_bytes());
                let what = format!(
                    "{:?} over {:?}",
                    String::from_utf8_lossy(bytes),
                    String::from_utf8_lossy(stale)
                );
                assert_eq!(window.modes(), src.modes(), "{what}");
                assert_eq!(window.cursor(), src.cursor(), "{what}");
                assert_eq!(window.pen(), src.pen(), "{what}");
                for row in 0..size.rows {
                    assert_eq!(window.line(row), src.line(row), "{what}");
                }
            }
        }
    }

    /// Origin mode (DECOM) is not restored while the scroll margins are untracked (ADR-0100, Codex review): the repaint leaves it
    /// off, and the cursor of a fresh and of a reconnecting window (stale margins and origin mode) is the node's right after it.
    #[test]
    fn origin_mode_is_left_off_and_the_cursor_lands_where_the_nodes_is() {
        let size = Size { cols: 16, rows: 10 };
        for prog in [&b"\x1b[4;8r\x1b[?6h\x1b[2;3H"[..], b"\x1b[4;8r\x1b[5;3H"] {
            let (src, r) = source(size, prog);
            assert!(!r.contains("\x1b[?6h"), "{r:?}");
            let mut fresh = AlacrittyTerminal::new(size);
            fresh.feed(r.as_bytes());
            let mut returning = AlacrittyTerminal::new(size);
            returning.feed(b"\x1b[3;8r\x1b[?6h");
            returning.feed(r.as_bytes());
            for window in [&fresh, &returning] {
                assert_eq!(window.cursor(), src.cursor());
                assert!(!window.modes().origin);
            }
        }
    }

    /// The known divergence this pins (ADR-0100: until the margins are tracked and restored): a program in origin mode with margins
    /// moves the cursor relative to them; the window, with origin mode off, puts it at the absolute row. When the follow-up restores
    /// the margins, this test changes to equality.
    #[test]
    fn a_live_origin_relative_move_still_diverges_while_the_margins_are_untracked() {
        let size = Size { cols: 16, rows: 10 };
        let (mut src, r) = source(size, b"\x1b[4;8r\x1b[?6h\x1b[2;3H");
        let mut fresh = AlacrittyTerminal::new(size);
        fresh.feed(r.as_bytes());
        let mut returning = AlacrittyTerminal::new(size);
        returning.feed(b"\x1b[3;8r\x1b[?6h");
        returning.feed(r.as_bytes());
        src.feed(b"\x1b[2;3H");
        fresh.feed(b"\x1b[2;3H");
        returning.feed(b"\x1b[2;3H");
        assert_eq!(src.cursor().row, 4, "relative to the top margin (row 3)");
        assert_eq!(
            fresh.cursor().row,
            1,
            "absolute: the margins are not restored"
        );
        assert_eq!(
            returning.cursor().row,
            1,
            "absolute: the margins are not restored"
        );
    }

    /// A window left in the line-drawing character set (`ESC ( 0`, or G1 shifted out) still draws the repaint's text as text.
    #[test]
    fn a_window_left_in_line_drawing_draws_the_repaint_as_text() {
        let size = Size { cols: 16, rows: 4 };
        let (src, r) = source(size, b"plain text qx");
        for stale in [&b"\x1b(0"[..], b"\x1b)0\x0e"] {
            let mut window = AlacrittyTerminal::new(size);
            window.feed(stale);
            window.feed(r.as_bytes());
            assert_eq!(window.line(0), src.line(0));
            window.feed(b"\r\nqx");
            assert_eq!(window.line(1), "qx", "live output after it is ASCII too");
        }
    }

    /// The pinned limitation of the keyboard flags (ADR-0100 known gaps): the repaint carries the current flags, not the program's
    /// stack, so when the program pops after a late attach, the node goes back to the flags below and the window to none.
    #[test]
    fn a_keyboard_pop_after_a_late_attach_still_diverges() {
        let size = Size { cols: 16, rows: 4 };
        let (mut src, r) = source(size, b"\x1b[>1u\x1b[>3u");
        let mut window = AlacrittyTerminal::new(size);
        window.feed(r.as_bytes());
        assert_eq!(window.modes().keyboard_flags, 3, "the current flags arrive");
        src.feed(b"\x1b[<u");
        window.feed(b"\x1b[<u");
        assert_eq!(src.modes().keyboard_flags, 1);
        assert_eq!(
            window.modes().keyboard_flags,
            0,
            "the stack below is not carried"
        );
    }

    /// The terminal client re-feeds the last repaint when its size arrives (`agora-tui` `preview.rs`): a second feed changes nothing.
    #[test]
    fn feeding_the_repaint_twice_changes_nothing() {
        let size = Size { cols: 16, rows: 4 };
        for bytes in [ALL_ON, ALL_OFF] {
            let (_, r) = source(size, bytes);
            let mut once = AlacrittyTerminal::new(size);
            once.feed(r.as_bytes());
            let mut twice = AlacrittyTerminal::new(size);
            twice.feed(r.as_bytes());
            twice.feed(r.as_bytes());
            assert_eq!(twice.modes(), once.modes());
            assert_eq!(twice.cursor(), once.cursor());
            assert_eq!(twice.bottom_lines(4), once.bottom_lines(4));
        }
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn an_update_wakes_every_registered_snapshot_waiter() {
        let handle = TerminalHandle {
            inner: inner(AlacrittyTerminal::new(Size { cols: 16, rows: 4 })),
            control: None,
            pid: 0,
            launch: 0,
            started: None,
            thread: None,
            held: false,
            launch_file: None,
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
