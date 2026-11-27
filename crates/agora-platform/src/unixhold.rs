//! ADR-0079 W2 step 1: the Unix primitives of the terminal holder, the process that owns the agents' PTY masters so they outlive their
//! node. Each part is tested by itself; the holder (`agora hold`), its link and the node's use of it are built on these:
//! - a program on a new PTY in a session of its own, whose master the holder keeps ([`HeldTerminal`]);
//! - the master as one passable thing: [`TerminalHandle`], sent over a unix socket with `SCM_RIGHTS` ([`crate::LocalStream`]);
//! - the peer of a unix socket ([`crate::Peer`]: pid and user);
//! - watching another process end ([`ProcessWatch`]: a pidfd on Linux, `kqueue` on macOS, the start time elsewhere);
//! - reading a terminal into a bounded buffer while no node is attached, on a loop that can be ended before a handover
//!   ([`TerminalReader`]).
//!
//! The holder never sets `O_NONBLOCK` on a master: the flag belongs to the open file description, which the node's copy shares, and the
//! node reads its copy with blocking reads. The reader waits with `poll` and reads only what `poll` said is there.
#![allow(dead_code)] // `agora hold` and the node call these; the platform tests also do.

use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::held::{Collected, HeldBuffer, HeldControl, HeldSignal};
use crate::unix::{cvt, status, winsize};
use crate::{ExitStatus, PtyConfig};

fn signal_number(signal: HeldSignal) -> libc::c_int {
    match signal {
        HeldSignal::Interrupt => libc::SIGINT,
        HeldSignal::Terminate => libc::SIGTERM,
        HeldSignal::Kill => libc::SIGKILL,
        HeldSignal::Hangup => libc::SIGHUP,
    }
}

// ----- poll -----

fn poll(fds: &mut [libc::pollfd], timeout_ms: libc::c_int) -> io::Result<usize> {
    loop {
        // SAFETY: a valid slice of pollfd for the length given.
        let r = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
        if r >= 0 {
            return Ok(r as usize);
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

fn pollfd(fd: RawFd) -> libc::pollfd {
    libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    }
}

/// One read of a master that `poll` said is ready: `Ok(None)` at the end of the terminal (the slave closed: `EIO` on Linux, 0 on macOS).
fn read_ready(master: &mut File, buf: &mut [u8]) -> io::Result<Option<usize>> {
    loop {
        match master.read(buf) {
            Ok(0) => return Ok(None),
            Ok(n) => return Ok(Some(n)),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if e.raw_os_error() == Some(libc::EIO) => return Ok(None),
            Err(e) => return Err(e),
        }
    }
}

// ----- the terminal -----

/// The master of a terminal as one thing that can be sent to another process (`SCM_RIGHTS`) and read and written there.
#[derive(Debug)]
pub struct TerminalHandle {
    pub(crate) fd: OwnedFd,
}

impl TerminalHandle {
    /// The master as a file: blocking reads and writes, as the node uses it.
    pub fn into_file(self) -> io::Result<File> {
        Ok(File::from(self.fd))
    }

    /// Windows only (M2.6): on Unix the master travels beside the frame (`SCM_RIGHTS`), never in it ([`crate::TERMINAL_IN_BAND`]).
    pub fn export_to<'a>(&self, _peer: &'a ProcessWatch) -> io::Result<ExportedTerminal<'a>> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    /// Windows only (M2.6), as [`Self::export_to`].
    pub fn import(_reference: &[u8]) -> io::Result<TerminalHandle> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// A terminal handed to the node inside a frame (Windows only, M2.6); never made on Unix.
pub struct ExportedTerminal<'a>(std::marker::PhantomData<&'a ()>);

impl ExportedTerminal<'_> {
    pub fn bytes(&self) -> &[u8] {
        &[]
    }
    pub fn delivered(self) {}
}

/// A program on a PTY the holder owns: its master, its child (so its exit status) and nothing else. The holder is the child's parent.
#[derive(Debug)]
pub struct HeldTerminal {
    master: File,
    child: Child,
    exit: Option<ExitStatus>,
}

impl HeldTerminal {
    /// Starts `config` in a session of its own, with exactly the environment in `config` (the holder's own is a stale snapshot of
    /// whoever started it, so none of it is passed on). On Linux the first process gets `SIGHUP` when the holder ends (ADR-0079 decision 8,
    /// with no node attached): call this from a thread that lives as long as the holder, for the kernel ties the signal to the
    /// thread that forked.
    pub fn start(config: &PtyConfig) -> io::Result<Self> {
        let (child, master) = crate::unix::start_child(config, None, true, true)?;
        Ok(Self {
            master,
            child,
            exit: None,
        })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Ends the agent's process group with `SIGKILL` and reaps its leader, so that no zombie is left (an agent that never got to be
    /// handed to a node).
    pub fn kill_and_reap(&mut self) {
        let _ = self.signal(HeldSignal::Kill);
        let _ = self.child.wait();
        self.exit = Some(ExitStatus {
            code: None,
            signal: Some(libc::SIGKILL),
        });
    }

    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        let ws = winsize(cols, rows);
        // SAFETY: a valid master and a pointer to a live winsize.
        cvt(unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &ws) }).map(|_| ())
    }

    /// Sends `signal` to the agent's process group. Nothing once the leader's end was collected ([`Self::try_wait`]): its process
    /// number may belong to another process then.
    pub fn signal(&self, signal: HeldSignal) -> io::Result<()> {
        if self.exit.is_some() {
            return Ok(());
        }
        // SAFETY: signalling the group this child leads (setsid), whose leader is not reaped yet.
        cvt(unsafe { libc::killpg(self.child.id() as libc::pid_t, signal_number(signal)) })
            .map(|_| ())
    }

    /// The agent's exit status once it ended, kept from then on.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.exit.is_none() {
            self.exit = self.child.try_wait()?.map(status);
        }
        Ok(self.exit)
    }

    /// A duplicate of the master for the node (`SCM_RIGHTS`); the holder keeps its own.
    pub fn handle(&self) -> io::Result<TerminalHandle> {
        Ok(TerminalHandle {
            fd: self.master.try_clone()?.into(),
        })
    }

    /// Starts reading this terminal into a bounded buffer (ADR-0079 decision 4).
    pub fn start_reader(&self) -> io::Result<TerminalReader> {
        TerminalReader::start(self.master.try_clone()?)
    }

    /// Reads what is waiting now into `buf`, never blocking. `true` at the end of the terminal. Only with no reader running.
    pub fn drain(&self, buf: &mut HeldBuffer) -> io::Result<bool> {
        let mut master = self.master.try_clone()?;
        let mut chunk = [0u8; 16 * 1024];
        loop {
            let mut fds = [pollfd(master.as_raw_fd())];
            if poll(&mut fds, 0)? == 0 {
                return Ok(false);
            }
            match read_ready(&mut master, &mut chunk)? {
                Some(n) => buf.push(&chunk[..n]),
                None => return Ok(true),
            }
        }
    }
}

impl Drop for HeldTerminal {
    fn drop(&mut self) {
        // A holder that lets go of an agent that still runs ends it, as the master closing would; and its leader is not left a zombie.
        if self.exit.is_none() {
            self.kill_and_reap();
        }
    }
}

// ----- the reader -----

struct Shared {
    buffer: HeldBuffer,
    ended: bool,
}

/// One thread reading one terminal into a bounded buffer. [`TerminalReader::end`] stops it and waits for it, so no read is in flight
/// when the buffer is handed over: a line read just then would otherwise be in neither the buffer nor the terminal.
pub struct TerminalReader {
    shared: Arc<Mutex<Shared>>,
    wake: File,
    thread: Option<JoinHandle<()>>,
}

impl TerminalReader {
    fn start(master: File) -> io::Result<Self> {
        let (wake_r, wake_w) = wake_pipe()?;
        let shared = Arc::new(Mutex::new(Shared {
            buffer: HeldBuffer::default(),
            ended: false,
        }));
        let theirs = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("agora-held-reader".into())
            .spawn(move || read_loop(master, wake_r, &theirs))?;
        Ok(Self {
            shared,
            wake: File::from(wake_w),
            thread: Some(thread),
        })
    }

    /// Whether the terminal ended while being read.
    pub fn ended(&self) -> bool {
        self.shared.lock().unwrap_or_else(|e| e.into_inner()).ended
    }

    /// Bytes kept so far.
    pub fn buffered(&self) -> usize {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .buffer
            .len()
    }

    /// Ends the read and waits until it has ended, then returns what it kept. Output not read yet stays in the terminal.
    pub fn end(mut self) -> Collected {
        self.stop();
        let mut s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        Collected {
            buffer: std::mem::take(&mut s.buffer),
            ended: s.ended,
        }
    }

    fn stop(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        // A full pipe cannot happen (one byte, once); a failed write only means the thread is gone already.
        let _ = io::Write::write(&mut &self.wake, &[1]);
        let _ = thread.join();
    }
}

impl Drop for TerminalReader {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A pipe that no program the holder starts can inherit: made close-on-exec in one step where the system allows, and otherwise under
/// the lock that serializes starting programs.
fn wake_pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut ends = [0 as libc::c_int; 2];
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // SAFETY: a valid out array of two descriptors.
        cvt(unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) })?;
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let _guard = crate::unix::spawn_lock();
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        // SAFETY: a valid out array of two descriptors.
        cvt(unsafe { libc::pipe(ends.as_mut_ptr()) })?;
    }
    // SAFETY: pipe returned two fresh descriptors that nothing else owns.
    let (r, w) = unsafe { (OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1])) };
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    for fd in [&r, &w] {
        // SAFETY: plain fcntl on a descriptor we own.
        cvt(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) })?;
    }
    Ok((r, w))
}

fn read_loop(mut master: File, wake: OwnedFd, shared: &Mutex<Shared>) {
    let mut chunk = vec![0u8; 16 * 1024];
    loop {
        let mut fds = [pollfd(master.as_raw_fd()), pollfd(wake.as_raw_fd())];
        if poll(&mut fds, -1).is_err() {
            break;
        }
        // The stop request wins over output that is waiting: that output stays in the terminal for the node.
        if fds[1].revents != 0 {
            break;
        }
        if fds[0].revents == 0 {
            continue;
        }
        match read_ready(&mut master, &mut chunk) {
            Ok(Some(n)) => shared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .buffer
                .push(&chunk[..n]),
            Ok(None) | Err(_) => {
                shared.lock().unwrap_or_else(|e| e.into_inner()).ended = true;
                break;
            }
        }
    }
}

// ----- watching a process -----

/// Another process (the node, or the holder), watched for its end. Opened once and checked against its start time on that handle.
pub struct ProcessWatch {
    pid: u32,
    started: u64,
    #[cfg(target_os = "linux")]
    pidfd: Option<OwnedFd>,
    #[cfg(target_os = "macos")]
    kq: Option<OwnedFd>,
}

impl ProcessWatch {
    /// Watches process `pid`, which must have started at `started` ([`crate::process_start_time`]); `NotFound` when no such process
    /// runs or another one has its pid now. The check follows the opening of the handle, so the two cannot name two processes.
    pub fn open(pid: u32, started: u64) -> io::Result<Self> {
        #[allow(unused_mut)]
        let mut watch = Self {
            pid,
            started,
            #[cfg(target_os = "linux")]
            pidfd: None,
            #[cfg(target_os = "macos")]
            kq: None,
        };
        #[cfg(target_os = "linux")]
        {
            // SAFETY: pidfd_open takes a pid and flags and returns a new descriptor or -1.
            let r = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
            if r >= 0 {
                // SAFETY: a fresh descriptor that nothing else owns.
                let fd = unsafe { OwnedFd::from_raw_fd(r as RawFd) };
                // SAFETY: plain fcntl on a descriptor we own.
                cvt(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) })?;
                watch.pidfd = Some(fd);
            } else {
                let e = io::Error::last_os_error();
                // No such process; any other failure (an old kernel, a seccomp filter) falls back to the start-time poll.
                if e.raw_os_error() == Some(libc::ESRCH) {
                    return Err(io::Error::from(io::ErrorKind::NotFound));
                }
            }
        }
        #[cfg(target_os = "macos")]
        {
            watch.kq = register_exit(pid)?;
        }
        if crate::process_start_time(pid) != Some(started) {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        Ok(watch)
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Whether the process has ended (an ended one nobody reaped yet counts), without waiting.
    pub fn ended(&self) -> bool {
        #[cfg(target_os = "linux")]
        if let Some(fd) = &self.pidfd {
            let mut fds = [pollfd(fd.as_raw_fd())];
            return poll(&mut fds, 0).is_ok_and(|n| n > 0);
        }
        #[cfg(target_os = "macos")]
        if let Some(kq) = &self.kq {
            if exit_seen(kq.as_raw_fd()) {
                return true;
            }
        }
        crate::process_start_time(self.pid) != Some(self.started)
    }

    /// Waits up to `timeout` for the process to end.
    pub fn wait(&self, timeout: Duration) -> bool {
        let end = std::time::Instant::now().checked_add(timeout);
        loop {
            if self.ended() {
                return true;
            }
            let sleep = match end {
                Some(end) => {
                    let left = end.saturating_duration_since(std::time::Instant::now());
                    if left.is_zero() {
                        return false;
                    }
                    left.min(Duration::from_millis(10))
                }
                None => Duration::from_millis(10),
            };
            std::thread::sleep(sleep);
        }
    }
}

#[cfg(target_os = "macos")]
fn register_exit(pid: u32) -> io::Result<Option<OwnedFd>> {
    // SAFETY: plain system call.
    let kq = unsafe { libc::kqueue() };
    if kq < 0 {
        return Ok(None);
    }
    // SAFETY: a fresh descriptor that nothing else owns.
    let kq = unsafe { OwnedFd::from_raw_fd(kq) };
    // SAFETY: an all-zero kevent is a valid value.
    let mut ev: libc::kevent = unsafe { std::mem::zeroed() };
    ev.ident = pid as libc::uintptr_t;
    ev.filter = libc::EVFILT_PROC;
    ev.flags = libc::EV_ADD | libc::EV_ONESHOT;
    ev.fflags = libc::NOTE_EXIT;
    // SAFETY: valid kqueue and one valid change; no event list requested.
    let r = unsafe {
        libc::kevent(
            kq.as_raw_fd(),
            &ev,
            1,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
        )
    };
    if r < 0 {
        let e = io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::ESRCH) {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        // Registering can fail for a process of another user; the start-time poll still works.
        return Ok(None);
    }
    Ok(Some(kq))
}

#[cfg(target_os = "macos")]
fn exit_seen(kq: RawFd) -> bool {
    // SAFETY: an all-zero kevent is a valid out-buffer.
    let mut ev: libc::kevent = unsafe { std::mem::zeroed() };
    let zero = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: a valid kqueue, no changes, one valid out event and a zero timeout.
    let r = unsafe { libc::kevent(kq, std::ptr::null(), 0, &mut ev, 1, &zero) };
    r > 0
}

// ----- sessions and peers -----

/// Closes every descriptor this process was handed at its start but 0, 1 and 2 (a pipe of its starter, another program's log). The holder
/// calls it first thing, before it opens anything of its own.
pub(crate) fn close_inherited_descriptors() {
    crate::unix::close_from(3);
}

/// Makes this process the leader of a session of its own (no terminal, not in the starter's group), so what ends the starter's session
/// or group does not end it. Already a session leader is fine. A process that leads a process group cannot call `setsid` (`EPERM`):
/// that is an error here, never ignored, for a holder left in its starter's session would end with it. Whoever starts the holder
/// must make sure it is not a group leader (start it detached, or fork once).
pub(crate) fn start_new_session() -> io::Result<()> {
    // SAFETY: plain system calls.
    unsafe {
        if libc::getsid(0) == libc::getpid() {
            return Ok(());
        }
        cvt(libc::setsid()).map(|_| ())
    }
}

/// The process and user at the other end of a connected unix socket, as the kernel recorded them when it connected.
pub(crate) fn peer_of(sock: RawFd) -> io::Result<crate::Peer> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let mut cred: libc::ucred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: a valid out buffer and its length.
        cvt(unsafe {
            libc::getsockopt(
                sock,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut len,
            )
        })?;
        // SAFETY: plain system call.
        let mine = unsafe { libc::geteuid() };
        Ok(crate::Peer {
            pid: cred.pid as u32,
            same_user: cred.uid == mine,
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let (mut uid, mut gid): (libc::uid_t, libc::gid_t) = (0, 0);
        // SAFETY: valid out pointers.
        cvt(unsafe { libc::getpeereid(sock, &mut uid, &mut gid) })?;
        let mut pid: libc::pid_t = 0;
        let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
        // SAFETY: a valid out buffer and its length. SOL_LOCAL is 0 and LOCAL_PEERPID 2 on macOS.
        cvt(unsafe {
            libc::getsockopt(sock, 0, 2, (&mut pid as *mut libc::pid_t).cast(), &mut len)
        })?;
        // SAFETY: plain system call.
        let mine = unsafe { libc::geteuid() };
        Ok(crate::Peer {
            pid: pid as u32,
            same_user: uid == mine,
        })
    }
}

// ----- descriptors over a unix socket -----

#[cfg(any(target_os = "linux", target_os = "android"))]
const SEND_FLAGS: libc::c_int = libc::MSG_NOSIGNAL;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
const SEND_FLAGS: libc::c_int = 0;

/// Room for one `SCM_RIGHTS` message of one descriptor, aligned for `cmsghdr`.
const CONTROL_WORDS: usize = 8;

/// Whether `sock`'s connection is shut in both directions, by the peer closing it or by a shutdown of this end: `poll(2)` with no
/// wait reports a hang-up (Linux: both directions shut; macOS: the end of the stream, `EV_EOF`), also while bytes the peer sent are
/// still to be read. This is the state in which macOS refuses a new socket timeout with `EINVAL`.
pub(crate) fn hung_up(sock: RawFd) -> io::Result<bool> {
    let mut p = libc::pollfd {
        fd: sock,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        // SAFETY: one valid pollfd that lives through the call; a zero timeout never blocks.
        let r = unsafe { libc::poll(&mut p, 1, 0) };
        if r >= 0 {
            return Ok(r > 0 && p.revents & libc::POLLHUP != 0);
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// Waits until `sock` is writable (or in error or hung up), at most `timeout`. `Ok(false)` on the time running out.
pub(crate) fn wait_writable(sock: RawFd, timeout: std::time::Duration) -> io::Result<bool> {
    wait_writable_polling(sock, timeout, libc::c_int::MAX)
}

/// [`wait_writable`], with each `poll(2)` waiting at most `max_poll_ms` (its argument is an `int` of milliseconds).
fn wait_writable_polling(
    sock: RawFd,
    timeout: std::time::Duration,
    max_poll_ms: libc::c_int,
) -> io::Result<bool> {
    // A timeout past what an `Instant` can hold (`Duration::MAX`) has no end: it waits until writable.
    let end = std::time::Instant::now().checked_add(timeout);
    loop {
        let left = end.map_or(timeout, |end| {
            end.saturating_duration_since(std::time::Instant::now())
        });
        // Rounded up to a whole millisecond, so the wait never ends before the time given.
        let ms = left.as_nanos().div_ceil(1_000_000).min(max_poll_ms as u128) as libc::c_int;
        let mut p = libc::pollfd {
            fd: sock,
            events: libc::POLLOUT,
            revents: 0,
        };
        // SAFETY: one valid pollfd that lives through the call.
        let r = unsafe { libc::poll(&mut p, 1, ms) };
        if r > 0 {
            return Ok(true);
        }
        if r == 0 {
            // One poll waits at most `max_poll_ms`: the time has run out only at the wait's own end (never, with no end).
            if end.is_some_and(|end| std::time::Instant::now() >= end) {
                return Ok(false);
            }
            continue;
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// One `sendmsg` of `bytes`, with `fd` attached to them when given. Returns how many bytes went; the descriptor went with them
/// when that is more than 0.
pub(crate) fn send_with_fd(sock: RawFd, bytes: &[u8], fd: Option<RawFd>) -> io::Result<usize> {
    let mut iov = libc::iovec {
        iov_base: bytes.as_ptr() as *mut libc::c_void,
        iov_len: bytes.len(),
    };
    let mut control = [0u64; CONTROL_WORDS];
    // SAFETY: an all-zero msghdr is a valid value.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    if let Some(fd) = fd {
        // SAFETY: CMSG_SPACE/CMSG_LEN are pure size computations.
        let space = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) } as usize;
        debug_assert!(space <= std::mem::size_of_val(&control));
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = space as _;
        // SAFETY: msg_control points at a zeroed, aligned buffer of `space` bytes, so the first header and its data fit.
        unsafe {
            let c = libc::CMSG_FIRSTHDR(&msg);
            (*c).cmsg_level = libc::SOL_SOCKET;
            (*c).cmsg_type = libc::SCM_RIGHTS;
            (*c).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
            std::ptr::copy_nonoverlapping(
                (&fd as *const RawFd).cast::<u8>(),
                libc::CMSG_DATA(c),
                std::mem::size_of::<RawFd>(),
            );
        }
    }
    loop {
        // SAFETY: `msg` and everything it points at live through the call.
        let r = unsafe { libc::sendmsg(sock, &msg, SEND_FLAGS) };
        if r >= 0 {
            return Ok(r as usize);
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// One `recvmsg` into `buf` that also collects a descriptor sent with the bytes (at most one is kept; any more are closed). Callers
/// read a message in exact sizes, so a descriptor sent with its first byte is never mixed up with the next message's.
pub(crate) fn recv_with_fd(sock: RawFd, buf: &mut [u8]) -> io::Result<(usize, Option<OwnedFd>)> {
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast(),
        iov_len: buf.len(),
    };
    let mut control = [0u64; CONTROL_WORDS];
    // SAFETY: an all-zero msghdr is a valid value.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr().cast();
    msg.msg_controllen = std::mem::size_of_val(&control) as _;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let flags = libc::MSG_CMSG_CLOEXEC;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let flags = 0;
    let n = loop {
        // SAFETY: `msg` and everything it points at live through the call.
        let r = unsafe { libc::recvmsg(sock, &mut msg, flags) };
        if r >= 0 {
            break r as usize;
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    };
    let mut got: Option<OwnedFd> = None;
    // SAFETY: the kernel filled `msg_control` with well-formed headers; each data area holds `cmsg_len - CMSG_LEN(0)` bytes.
    unsafe {
        let mut c = libc::CMSG_FIRSTHDR(&msg);
        while !c.is_null() {
            if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                let bytes = (*c).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
                let data = libc::CMSG_DATA(c);
                for i in 0..bytes / std::mem::size_of::<RawFd>() {
                    let mut fd: RawFd = -1;
                    std::ptr::copy_nonoverlapping(
                        data.add(i * std::mem::size_of::<RawFd>()),
                        (&mut fd as *mut RawFd).cast::<u8>(),
                        std::mem::size_of::<RawFd>(),
                    );
                    let owned = OwnedFd::from_raw_fd(fd);
                    if got.is_none() {
                        // macOS has no MSG_CMSG_CLOEXEC; where it has, this only repeats it.
                        let _ = libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
                        got = Some(owned);
                    } // else: dropped, which closes it
                }
            }
            c = libc::CMSG_NXTHDR(&msg, c);
        }
    }
    if msg.msg_flags & libc::MSG_CTRUNC != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "descriptors were cut off",
        ));
    }
    Ok((n, got))
}

// ----- a held agent's terminal, on the node's side -----

/// The node's side of a held agent (ADR-0079 decision 3): the master the holder handed over, read and written by the node itself, as
/// [`crate::unix`] does for an agent the node started. Nothing passes through the holder per byte. What only the agent's parent knows
/// (its exit status, a signal to its group) is asked of the holder through [`HeldControl`].
struct HeldPty {
    master: File,
    pid: u32,
    control: Arc<dyn HeldControl>,
    output: std::sync::mpsc::Receiver<Vec<u8>>,
    shared: Arc<HeldShared>,
    reader: Option<JoinHandle<()>>,
    status: Option<ExitStatus>,
}

/// What a [`HeldPty`] and its [`crate::PtyControl`] share.
struct HeldShared {
    /// The node is stopping and leaves the agent running: nothing ends it from here on.
    detached: std::sync::atomic::AtomicBool,
    /// Ends the reader's wait (the write end of a pipe the reader polls with the master).
    wake: File,
}

impl HeldShared {
    fn detach(&self) {
        self.detached
            .store(true, std::sync::atomic::Ordering::Release);
        // One byte once; a full pipe cannot happen, and a failed write only means the reader is gone already.
        let _ = io::Write::write(&mut &self.wake, &[1]);
    }

    fn detached(&self) -> bool {
        self.detached.load(std::sync::atomic::Ordering::Acquire)
    }
}

const HELD_CHUNK: usize = 16 * 1024;
const HELD_QUEUE: usize = 64;

/// The end of an agent whose holder is gone: nothing is known of how it ended.
const LOST: ExitStatus = ExitStatus {
    code: None,
    signal: None,
};

/// Wraps the master the holder handed over as a [`crate::Pty`] of the node's terminal actor. The reader waits with `poll` on the master
/// and a wake pipe, so that [`crate::PtyControl::detach`] and dropping it end the read (a node that lets go must not take output meant
/// for the next one); it reads the master blocking and in the same chunks as an agent the node started.
pub fn held_pty(
    master: TerminalHandle,
    pid: u32,
    control: Arc<dyn HeldControl>,
) -> io::Result<Box<dyn crate::Pty>> {
    let master = master.into_file()?;
    let (wake_r, wake_w) = wake_pipe()?;
    let mut read_side = master.try_clone()?;
    let (tx, output) = std::sync::mpsc::sync_channel::<Vec<u8>>(HELD_QUEUE);
    let reader = std::thread::Builder::new()
        .name("agora-held-pty-reader".into())
        .spawn(move || {
            let mut buf = vec![0u8; HELD_CHUNK];
            loop {
                let mut fds = [pollfd(read_side.as_raw_fd()), pollfd(wake_r.as_raw_fd())];
                if poll(&mut fds, -1).is_err() || fds[1].revents != 0 {
                    break;
                }
                if fds[0].revents == 0 {
                    continue;
                }
                match read_ready(&mut read_side, &mut buf) {
                    Ok(Some(n)) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Ok(None) | Err(_) => break,
                }
            }
        })?;
    Ok(Box::new(HeldPty {
        master,
        pid,
        control,
        output,
        shared: Arc::new(HeldShared {
            detached: std::sync::atomic::AtomicBool::new(false),
            wake: File::from(wake_w),
        }),
        reader: Some(reader),
        status: None,
    }))
}

fn kill_through_holder(shared: &HeldShared, control: &dyn HeldControl) -> io::Result<()> {
    if shared.detached() {
        return Ok(());
    }
    control.signal(HeldSignal::Kill)
}

impl crate::Pty for HeldPty {
    fn output(&self) -> &std::sync::mpsc::Receiver<Vec<u8>> {
        &self.output
    }

    fn write_input(&mut self, bytes: &[u8]) -> io::Result<()> {
        io::Write::write_all(&mut self.master, bytes)
    }

    fn resize(&mut self, cols: u16, rows: u16) -> io::Result<()> {
        let ws = winsize(cols, rows);
        // SAFETY: a valid master and a pointer to a live winsize.
        cvt(unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &ws) }).map(|_| ())
    }

    fn pid(&self) -> u32 {
        self.pid
    }

    fn kill(&mut self) -> io::Result<()> {
        if self.status.is_some() {
            return Ok(());
        }
        kill_through_holder(&self.shared, self.control.as_ref())
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.status.is_none() {
            if self.shared.detached() {
                // The node lets go: this ends the actor, and is not an end of the agent (nothing is reported of it).
                return Ok(Some(LOST));
            }
            self.status = match self.control.exit_status() {
                Ok(s) => s,
                // The holder is gone: the agent's own end cannot be known.
                Err(_) => Some(LOST),
            };
        }
        Ok(self.status)
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        loop {
            if let Some(s) = self.try_wait()? {
                return Ok(s);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn foreground_pid(&self) -> io::Result<u32> {
        // SAFETY: a valid master.
        let pgid = cvt(unsafe { libc::tcgetpgrp(self.master.as_raw_fd()) })?;
        Ok(pgid as u32)
    }

    fn control(&self) -> io::Result<crate::PtyControl> {
        Ok(crate::PtyControl {
            inner: Arc::new(HeldControlOps {
                input: Mutex::new(self.master.try_clone()?),
                master: self.master.try_clone()?,
                pid: self.pid,
                control: Arc::clone(&self.control),
                shared: Arc::clone(&self.shared),
            }),
        })
    }
}

impl Drop for HeldPty {
    fn drop(&mut self) {
        // Ends the read first, so that no copy of the master outlives this value (a copy that kept reading would take output meant for
        // the next node). Does not end the agent: that is `kill`, which the terminal's own drop calls first and which does nothing once
        // detached.
        self.output = std::sync::mpsc::sync_channel(1).1;
        self.shared.detach();
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
    }
}

/// [`crate::PtyControl`] of a held agent: input and size on the node's own copy of the master, a kill through the holder.
struct HeldControlOps {
    input: Mutex<File>,
    master: File,
    pid: u32,
    control: Arc<dyn HeldControl>,
    shared: Arc<HeldShared>,
}

impl crate::PtyControlOps for HeldControlOps {
    fn write_input(&self, bytes: &[u8]) -> io::Result<()> {
        let mut f = self.input.lock().unwrap_or_else(|e| e.into_inner());
        io::Write::write_all(&mut *f, bytes)
    }

    fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        let ws = winsize(cols, rows);
        // SAFETY: a valid master and a pointer to a live winsize.
        cvt(unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &ws) }).map(|_| ())
    }

    fn kill(&self) -> io::Result<()> {
        kill_through_holder(&self.shared, self.control.as_ref())
    }

    fn foreground_pid(&self) -> io::Result<u32> {
        // SAFETY: a valid master.
        let pgid = cvt(unsafe { libc::tcgetpgrp(self.master.as_raw_fd()) })?;
        Ok(pgid as u32)
    }

    fn pid(&self) -> u32 {
        self.pid
    }

    fn detach(&self) {
        self.shared.detach();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::held::HELD_BUFFER;

    /// A bounded wait longer than one poll may last keeps polling until its own deadline (F78 review): with `poll(2)` capped
    /// at 20 ms, a 200 ms wait on a full socket still takes the whole 200 ms.
    #[test]
    fn a_wait_longer_than_one_poll_lasts_until_its_deadline() {
        use std::io::Write;
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;
        let (mut a, _b) = UnixStream::pair().unwrap();
        a.set_nonblocking(true).unwrap();
        let chunk = vec![0u8; 64 << 10];
        while a.write(&chunk).is_ok() {}
        let want = std::time::Duration::from_millis(200);
        let began = std::time::Instant::now();
        assert!(!wait_writable_polling(a.as_raw_fd(), want, 20).unwrap());
        let took = began.elapsed();
        assert!(took >= want, "ended early: {took:?}");
    }

    /// Issue #830: the probe behind `LocalStream::set_read_timeout_or_closed` tells a connection shut in both directions apart from
    /// one that is open, with or without bytes to read.
    #[test]
    fn a_hang_up_is_seen_only_once_the_connection_is_shut() {
        use std::io::Write;
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;
        let (a, mut b) = UnixStream::pair().unwrap();
        assert!(!hung_up(a.as_raw_fd()).unwrap(), "open and idle");
        b.write_all(b"buffered").unwrap();
        assert!(!hung_up(a.as_raw_fd()).unwrap(), "open with bytes to read");
        drop(b);
        assert!(
            hung_up(a.as_raw_fd()).unwrap(),
            "the peer closed, bytes still to read"
        );
        let (c, d) = UnixStream::pair().unwrap();
        drop(d);
        assert!(hung_up(c.as_raw_fd()).unwrap(), "the peer closed");
        let (e, _f) = UnixStream::pair().unwrap();
        e.shutdown(std::net::Shutdown::Write).unwrap();
        assert!(
            !hung_up(e.as_raw_fd()).unwrap(),
            "only this end's writing shut"
        );
        // A fresh pair: macOS refuses a second shutdown of a half-shut socket (`ENOTCONN`).
        let (g, _h) = UnixStream::pair().unwrap();
        g.shutdown(std::net::Shutdown::Both).unwrap();
        assert!(hung_up(g.as_raw_fd()).unwrap(), "this end shut both ways");
    }

    #[test]
    fn the_buffer_drops_the_oldest_first() {
        let mut b = HeldBuffer::with_capacity(8);
        b.push(b"abcd");
        b.push(b"efgh");
        assert!(!b.dropped());
        assert_eq!(b.clone().into_vec(), b"abcdefgh");
        b.push(b"ij");
        assert!(b.dropped());
        assert_eq!(b.clone().into_vec(), b"cdefghij");
        // A write bigger than the bound keeps its own tail.
        b.push(b"0123456789AB");
        assert_eq!(b.into_vec(), b"456789AB");
    }

    #[test]
    fn the_buffer_is_one_megabyte() {
        let mut b = HeldBuffer::default();
        b.push(&vec![7u8; HELD_BUFFER]);
        assert!(!b.dropped());
        b.push(b"x");
        assert!(b.dropped());
        assert_eq!(b.len(), HELD_BUFFER);
    }

    #[test]
    fn a_descriptor_travels_with_its_bytes_and_no_other_message_gets_it() {
        use std::os::unix::net::UnixStream;
        let (a, b) = UnixStream::pair().unwrap();
        let file = tempfile_like();
        // Two messages in a row: only the first carries a descriptor.
        assert_eq!(
            send_with_fd(a.as_raw_fd(), b"one!", Some(file.as_raw_fd())).unwrap(),
            4
        );
        assert_eq!(send_with_fd(a.as_raw_fd(), b"two!", None).unwrap(), 4);
        let mut buf = [0u8; 4];
        let (n, fd) = recv_with_fd(b.as_raw_fd(), &mut buf).unwrap();
        assert_eq!((n, &buf), (4, b"one!"));
        let fd = fd.expect("the descriptor");
        let (n, none) = recv_with_fd(b.as_raw_fd(), &mut buf).unwrap();
        assert_eq!((n, &buf), (4, b"two!"));
        assert!(none.is_none());
        // It is the same open file: what the sender wrote through its end is read through the copy.
        let mut copy = File::from(fd);
        let mut text = String::new();
        io::Seek::seek(&mut copy, io::SeekFrom::Start(0)).unwrap();
        copy.read_to_string(&mut text).unwrap();
        assert_eq!(text, "shared");
    }

    fn tempfile_like() -> File {
        // A memfd-free stand-in that works on every Unix: an unlinked file in the temp folder.
        let path = std::env::temp_dir().join(format!(
            "agora-fd-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        io::Write::write_all(&mut f, b"shared").unwrap();
        f
    }

    #[test]
    fn the_peer_is_this_process() {
        use std::os::unix::net::UnixStream;
        let (a, _b) = UnixStream::pair().unwrap();
        let p = peer_of(a.as_raw_fd()).unwrap();
        assert_eq!(p.pid, std::process::id());
        assert!(p.same_user);
    }

    #[test]
    fn a_watch_sees_a_process_end_and_refuses_a_wrong_start_time() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let started = crate::process_start_time(child.id()).unwrap();
        assert!(ProcessWatch::open(child.id(), started + 1).is_err());
        let w = ProcessWatch::open(child.id(), started).unwrap();
        assert!(!w.ended());
        child.kill().unwrap();
        assert!(w.wait(Duration::from_secs(5)));
        let _ = child.wait();
        assert!(w.ended());
        assert!(ProcessWatch::open(child.id(), started).is_err());
    }

    #[test]
    fn process_watch_wait_does_not_panic_on_duration_max() {
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg("sleep 0.05")
            .spawn()
            .unwrap();
        let started = crate::process_start_time(child.id()).unwrap();
        let w = ProcessWatch::open(child.id(), started).unwrap();
        assert!(w.wait(Duration::MAX));
        let _ = child.wait();
    }

    #[test]
    fn an_agent_the_holder_starts_sees_no_descriptor_it_was_handed() {
        // Two descriptors that are not close-on-exec: what a starter's pipes look like to a process it started.
        let mut ends = [0 as libc::c_int; 2];
        // SAFETY: a valid out array of two descriptors.
        assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0);
        let script = format!(
            "for f in {} {}; do if [ -e /dev/fd/$f ]; then echo OPEN$f; fi; done; echo DONE; sleep 30",
            ends[0], ends[1]
        );
        let cfg = PtyConfig {
            argv: vec!["sh".into(), "-c".into(), script],
            cwd: None,
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
            cols: 80,
            rows: 24,
            limits: crate::ProcessLimits::default(),
            sandbox: None,
            identity: None,
            env_remove: Vec::new(),
        };
        let t = HeldTerminal::start(&cfg).unwrap();
        let r = t.start_reader().unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        let text = String::from_utf8_lossy(&r.end().buffer.into_vec()).into_owned();
        assert!(text.contains("DONE"), "the script did not run: {text:?}");
        assert!(
            !text.contains("OPEN"),
            "the agent was handed a descriptor: {text:?}"
        );
        t.signal(HeldSignal::Kill).unwrap();
    }

    #[test]
    fn a_reader_buffers_and_what_it_ends_with_is_all_it_read() {
        let cfg = PtyConfig {
            argv: vec!["sh".into(), "-c".into(), "printf one; sleep 30".into()],
            cwd: None,
            env: Vec::new(),
            cols: 80,
            rows: 24,
            limits: crate::ProcessLimits::default(),
            sandbox: None,
            identity: None,
            env_remove: Vec::new(),
        };
        let t = HeldTerminal::start(&cfg).unwrap();
        let r = t.start_reader().unwrap();
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while r.buffered() < 3 && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        let c = r.end();
        assert_eq!(c.buffer.into_vec(), b"one");
        assert!(!c.ended);
        t.signal(HeldSignal::Kill).unwrap();
    }

    /// A holder-side stand-in for a node's [`HeldControl`]: it answers nothing about the agent's end and refuses every signal (so a test sees one that should not have been sent).
    struct Quiet;

    impl HeldControl for Quiet {
        fn signal(&self, _: HeldSignal) -> io::Result<()> {
            Err(io::Error::other("asked to signal"))
        }
        fn exit_status(&self) -> io::Result<Option<ExitStatus>> {
            Ok(None)
        }
    }

    fn sh_config(script: &str) -> PtyConfig {
        PtyConfig {
            argv: vec!["sh".into(), "-c".into(), script.into()],
            cwd: None,
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
            cols: 80,
            rows: 24,
            limits: crate::ProcessLimits::default(),
            sandbox: None,
            identity: None,
            env_remove: Vec::new(),
        }
    }

    /// A held terminal as the node reads and writes it: its output arrives, its input reaches the program, and the end of the terminal
    /// closes the output channel.
    #[test]
    fn a_held_terminal_reads_and_writes_like_one_the_node_started() {
        let t = HeldTerminal::start(&sh_config("echo first; read x; echo got-$x")).unwrap();
        let mut pty = held_pty(t.handle().unwrap(), t.pid(), Arc::new(Quiet)).unwrap();
        let mut seen = Vec::new();
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while !String::from_utf8_lossy(&seen).contains("first") {
            assert!(std::time::Instant::now() < end, "{seen:?}");
            if let Ok(c) = pty.output().recv_timeout(Duration::from_millis(50)) {
                seen.extend(c);
            }
        }
        pty.write_input(b"abc\n").unwrap();
        while !String::from_utf8_lossy(&seen).contains("got-abc") {
            assert!(
                std::time::Instant::now() < end,
                "{:?}",
                String::from_utf8_lossy(&seen)
            );
            if let Ok(c) = pty.output().recv_timeout(Duration::from_millis(50)) {
                seen.extend(c);
            }
        }
        // The program ended: the channel closes.
        while pty.output().recv_timeout(Duration::from_millis(50))
            != Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        {
            assert!(std::time::Instant::now() < end);
        }
    }

    /// Letting go of a held terminal ends its read and closes the node's copy of the master (a copy that kept reading would take output
    /// A pipe agent that is stopped (`SIGSTOP`) has not ended: the holder neither reports an end nor ends its group (review of #1108).
    #[test]
    fn a_stopped_pipe_agent_is_not_taken_for_ended() {
        let mut p = HeldProcess::start(
            &["sleep".to_string(), "30".to_string()],
            None,
            &[(
                "PATH".to_string(),
                std::env::var("PATH").unwrap_or_default(),
            )],
        )
        .unwrap();
        let pid = p.pid() as libc::pid_t;
        // SAFETY: plain system calls on a child of this test.
        unsafe { libc::kill(pid, libc::SIGSTOP) };
        let end = std::time::Instant::now() + Duration::from_millis(500);
        while std::time::Instant::now() < end {
            assert!(
                p.try_wait().unwrap().is_none(),
                "a stopped agent was taken for ended"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        // SAFETY: as above.
        assert_eq!(
            unsafe { libc::kill(pid, libc::SIGCONT) },
            0,
            "the stopped agent was ended"
        );
        p.kill_and_reap();
    }

    /// meant for the next node), and ends nothing: the agent runs on and its terminal can be read again.
    #[test]
    fn letting_go_of_a_held_terminal_ends_the_read_and_not_the_agent() {
        let mut t =
            HeldTerminal::start(&sh_config("while :; do echo tick; sleep 0.05; done")).unwrap();
        let pty = held_pty(t.handle().unwrap(), t.pid(), Arc::new(Quiet)).unwrap();
        let ctl = pty.control().unwrap();
        ctl.detach();
        // A kill after the node let go is nothing.
        ctl.kill().unwrap();
        drop(pty);
        assert!(t.try_wait().unwrap().is_none(), "the agent ended");
        // A new reader gets the output: nothing of it was taken by the one that let go (it is no longer reading).
        let mut again = held_pty(t.handle().unwrap(), t.pid(), Arc::new(Quiet)).unwrap();
        let c = again.output().recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(String::from_utf8_lossy(&c).contains("tick"));
        let _ = again.kill();
        t.signal(HeldSignal::Kill).unwrap();
    }

    /// ADR-0022: reading a terminal the node took over costs what reading one it started costs. Run with
    /// `cargo test -p agora-platform --release held_terminal_throughput -- --ignored --nocapture`; the numbers are in the PR.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn held_terminal_throughput_matches_a_terminal_the_node_started() {
        const MIB: usize = 64;
        let script = format!("head -c {} /dev/zero | tr '\\0' x", MIB << 20);
        let drain = |pty: &mut dyn crate::Pty| -> (usize, Duration) {
            let began = std::time::Instant::now();
            let mut n = 0;
            while let Ok(c) = pty.output().recv_timeout(Duration::from_secs(20)) {
                n += c.len();
            }
            (n, began.elapsed())
        };
        let mut rows = Vec::new();
        for round in 0..5 {
            let mut own = crate::spawn(&sh_config(&script)).unwrap();
            let (n_own, t_own) = drain(own.as_mut());
            let t = HeldTerminal::start(&sh_config(&script)).unwrap();
            let mut held = held_pty(t.handle().unwrap(), t.pid(), Arc::new(Quiet)).unwrap();
            let (n_held, t_held) = drain(held.as_mut());
            let mib = |n: usize, d: Duration| n as f64 / (1 << 20) as f64 / d.as_secs_f64();
            println!(
                "round {round}: node-started {n_own} bytes {:.0} MiB/s; held {n_held} bytes {:.0} MiB/s",
                mib(n_own, t_own),
                mib(n_held, t_held)
            );
            rows.push((mib(n_own, t_own), mib(n_held, t_held)));
        }
        let mean = |f: fn(&(f64, f64)) -> f64| rows.iter().map(f).sum::<f64>() / rows.len() as f64;
        println!(
            "mean: node-started {:.0} MiB/s, held {:.0} MiB/s",
            mean(|r| r.0),
            mean(|r| r.1)
        );
    }
}

// ----- a program on pipes (ADR-0093) -----

/// A program the holder runs on a pair of pipes instead of a terminal (a structured agent: an adapter or an ACP bridge): its input,
/// its output, its child (so its exit status) and nothing else. It leads a session and so a process group of its own, like a terminal
/// agent. Its stderr is the holder's own (the holder's log). The holder is the child's parent.
#[derive(Debug)]
pub struct HeldProcess {
    child: Child,
    io: Option<(std::process::ChildStdin, std::process::ChildStdout)>,
    exit: Option<ExitStatus>,
}

impl HeldProcess {
    /// Starts `argv` with exactly `env` (the holder's own environment is a stale snapshot of whoever started it and is never passed
    /// on), in `cwd` when given. Every descriptor the holder was handed beyond 0, 1 and 2 is closed on exec.
    pub fn start(
        argv: &[String],
        cwd: Option<&std::path::Path>,
        env: &[(String, String)],
    ) -> io::Result<Self> {
        let Some((program, args)) = argv.split_first() else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "argv is empty"));
        };
        let mut cmd = std::process::Command::new(program);
        cmd.args(args);
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        cmd.env_clear();
        cmd.envs(env.iter().map(|(k, v)| (k, v)));
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit());
        // SAFETY: runs between fork and exec and only calls async-signal-safe functions (the descriptor marking, setsid).
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(&mut cmd, || {
                crate::unix::cloexec_from(3);
                cvt(libc::setsid())?;
                Ok(())
            });
        }
        let mut child = cmd.spawn()?;
        let io = child.stdin.take().zip(child.stdout.take());
        let Some(io) = io else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("pipes missing"));
        };
        Ok(Self {
            child,
            io: Some(io),
            exit: None,
        })
    }

    /// The pipes, once: the agent's input and output.
    pub fn take_io(&mut self) -> Option<(std::process::ChildStdin, std::process::ChildStdout)> {
        self.io.take()
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Sends `signal` to the agent's process group. Nothing once the leader's end was collected ([`Self::try_wait`]).
    pub fn signal(&self, signal: HeldSignal) -> io::Result<()> {
        if self.exit.is_some() {
            return Ok(());
        }
        // SAFETY: signalling the group this child leads (setsid), whose leader is not reaped yet.
        cvt(unsafe { libc::killpg(self.child.id() as libc::pid_t, signal_number(signal)) })
            .map(|_| ())
    }

    /// The agent's exit status once it ended, kept from then on. **When the leader has ended, whatever else it left in its group (and,
    /// on Linux, in the rest of its session) is ended with it** (`SIGKILL`, at once, in this call): an adapter that ended or crashed must
    /// not leave processes the holder can no longer name, which would also hold its output open for good. They are signalled while the
    /// leader has ended but is **not yet collected** (`waitid` with `WNOWAIT`): until then its pid, which is also the number of its group
    /// and its session, cannot be given to any other process, so nothing outside the agent can be signalled. Only then is the leader
    /// collected, and the holder never signals it again afterwards ([`Self::signal`] does nothing once the end is collected).
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.exit.is_none() {
            let pid = self.child.id() as libc::pid_t;
            if ended_unreaped(pid)? {
                // SAFETY: the group this child led (setsid), whose leader has ended but is not collected: the number is still its own.
                // An error (no member left) is the usual case.
                let _ = unsafe { libc::killpg(pid, libc::SIGKILL) };
                kill_rest_of_session(pid);
            }
            self.exit = self.child.try_wait()?.map(status);
        }
        Ok(self.exit)
    }

    /// Ends the agent's process group with `SIGKILL` and reaps its leader, so that no zombie is left.
    pub fn kill_and_reap(&mut self) {
        let _ = self.signal(HeldSignal::Kill);
        let _ = self.child.wait();
        self.exit = Some(ExitStatus {
            code: None,
            signal: Some(libc::SIGKILL),
        });
    }
}

/// `siginfo_t.si_code` of a child that ended: exited, killed by a signal, killed with a core dump (the same values on Linux and macOS,
/// which the `libc` crate names on Linux only).
const CLD_EXITED: libc::c_int = 1;
const CLD_KILLED: libc::c_int = 2;
const CLD_DUMPED: libc::c_int = 3;

/// Whether the child `pid` has ended, without collecting it (`waitid` with `WEXITED | WNOHANG | WNOWAIT`): an ended child stays a
/// zombie, so its pid (and the group and session it led) stay its own until it is collected.
fn ended_unreaped(pid: libc::pid_t) -> io::Result<bool> {
    let Ok(id) = libc::id_t::try_from(pid) else {
        return Ok(false);
    };
    loop {
        // SAFETY: an all-zero siginfo_t is a valid out-buffer.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: a valid out-pointer; WNOWAIT leaves the child to be collected by `Child::try_wait`.
        let r = unsafe {
            libc::waitid(
                libc::P_PID,
                id,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if r == 0 {
            // SAFETY: filled in by a successful waitid; zero when the child has not ended (WNOHANG).
            let pid = unsafe { info.si_pid() };
            // Only an end counts: a child that was stopped (`SIGSTOP`, a debugger) is never taken for ended, whatever a system's
            // `waitid` reports (with only `WEXITED` neither Linux nor macOS reports another state: a defence, which costs nothing).
            return Ok(pid != 0 && [CLD_EXITED, CLD_KILLED, CLD_DUMPED].contains(&info.si_code));
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::EINTR) => continue,
            // Already collected (only `Child::try_wait` below does that): there is nothing left to signal safely.
            Some(libc::ECHILD) => return Ok(false),
            _ => return Err(e),
        }
    }
}

/// `SIGKILL` to every process still in the session `sid`: the session the holder made for a pipe agent (`setsid`), whose leader has
/// ended but is not collected yet. What the agent moved into process groups of their own (job control, `setpgid`) is not in its group but is
/// still in its session, and would otherwise run on unnamed and hold the agent's output open. The session's number is the uncollected
/// leader's pid, so no other session can have it. Linux only (it reads `/proc`); elsewhere only the group is ended. A process
/// that left the session (`setsid`) is not found.
fn kill_rest_of_session(sid: libc::pid_t) {
    #[cfg(target_os = "linux")]
    {
        let Ok(dir) = std::fs::read_dir("/proc") else {
            return;
        };
        for entry in dir.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<libc::pid_t>().ok())
            else {
                continue;
            };
            // SAFETY: plain system calls with integer arguments; a process that is gone is `ESRCH`, which is ignored.
            if unsafe { libc::getsid(pid) } == sid {
                let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = sid;
}

impl Drop for HeldProcess {
    fn drop(&mut self) {
        if self.exit.is_none() {
            self.kill_and_reap();
        }
    }
}
