//! PTY on Unix (Linux and macOS) via `openpty`.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::{ExitStatus, Pty, PtyConfig};

pub(crate) const NULL_DEVICE: &str = "/dev/null";

const CHUNK: usize = 16 * 1024;
/// Bounded queue between the reader thread and the consumer: backpressure reaches the process.
const QUEUE: usize = 64;

struct UnixPty {
    child: Child,
    writer: File,
    output: Receiver<Vec<u8>>,
    reader: Option<JoinHandle<()>>,
    exited: bool,
    /// Set once the child is reaped; shared with [`UnixControl`] so a late `kill` cannot hit a recycled pid.
    reaped: Arc<AtomicBool>,
}

/// [`crate::PtyControl`] over duplicated master descriptors.
struct UnixControl {
    /// Serializes concurrent input writes; `resize` and `kill` do not take it.
    input: Mutex<File>,
    master: File,
    pid: u32,
    reaped: Arc<AtomicBool>,
}

impl crate::PtyControlOps for UnixControl {
    fn write_input(&self, bytes: &[u8]) -> io::Result<()> {
        let mut f = self.input.lock().unwrap_or_else(|e| e.into_inner());
        f.write_all(bytes)
    }

    fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        let ws = winsize(cols, rows);
        // SAFETY: valid master fd and a pointer to a live winsize.
        cvt(unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &ws) }).map(|_| ())
    }

    fn kill(&self) -> io::Result<()> {
        if self.reaped.load(Ordering::Acquire) {
            return Ok(());
        }
        kill_group(self.pid)
    }

    fn foreground_pid(&self) -> io::Result<u32> {
        // SAFETY: valid master fd.
        let pgid = cvt(unsafe { libc::tcgetpgrp(self.master.as_raw_fd()) })?;
        Ok(pgid as u32)
    }

    fn pid(&self) -> u32 {
        self.pid
    }
}

fn kill_group(pid: u32) -> io::Result<()> {
    // SAFETY: signalling a process group we created with setsid.
    let r = unsafe { libc::killpg(pid as libc::pid_t, libc::SIGKILL) };
    if r < 0 {
        let e = io::Error::last_os_error();
        // The group is already gone: nothing left to kill.
        if e.raw_os_error() != Some(libc::ESRCH) {
            return Err(e);
        }
    }
    Ok(())
}

pub(crate) fn winsize(cols: u16, rows: u16) -> libc::winsize {
    libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

/// `ELOOP`: too many levels of symbolic links (a link to itself, or a ring of links), or a link refused by `O_NOFOLLOW`.
pub(crate) fn link_loop(e: &io::Error) -> bool {
    e.raw_os_error() == Some(libc::ELOOP)
}

pub(crate) fn cvt(r: libc::c_int) -> io::Result<libc::c_int> {
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(r)
    }
}

/// Set a soft and hard limit for the calling process. Async-signal-safe.
fn set_limit(resource: u32, value: u64) -> io::Result<()> {
    let lim = libc::rlimit {
        rlim_cur: value as libc::rlim_t,
        rlim_max: value as libc::rlim_t,
    };
    // SAFETY: `lim` is a valid rlimit; the resource constant comes from libc.
    cvt(unsafe { libc::setrlimit(resource as _, &lim) }).map(|_| ())
}

pub(crate) fn local_listen(path: &std::path::Path) -> io::Result<crate::LocalListener> {
    use std::os::unix::fs::FileTypeExt;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    let not_a_socket = || {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the socket path exists and is not a socket",
        )
    };
    // Checked before the lock too, so a mistyped `--socket` leaves no lock file behind.
    if std::fs::symlink_metadata(path).is_ok_and(|m| !m.file_type().is_socket()) {
        return Err(not_a_socket());
    }
    // Held for as long as the listener lives: a refused connection does not prove a socket stale (a live server whose queue is full
    // refuses at once on macOS), this lock does.
    let lock = lock_beside(path)?;
    // A socket file nobody serves is left over from a crash: replace it. A live one is an error, and so is anything that is
    // not a socket at all (a mistyped `--socket` must not delete somebody's file).
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if !meta.file_type().is_socket() {
            return Err(not_a_socket());
        }
        match UnixStream::connect(path) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "socket is in use by another process",
                ))
            }
            Err(_) => std::fs::remove_file(path)?,
        }
    }
    let inner = UnixListener::bind(path)?;
    // Only the owner may connect: the control socket can start and stop agents.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(crate::LocalListener {
        inner,
        path: path.to_path_buf(),
        _lock: lock,
    })
}

/// `<path>.lock`, locked (`flock`) by the listener serving `path`; `AddrInUse` while another process holds it. The file is left in
/// place when the listener ends: removing it would let two starters lock two different files.
fn lock_beside(path: &std::path::Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(name)?;
    // SAFETY: flock on a descriptor we own.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let e = io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "socket is in use by another process",
            ));
        }
        return Err(e);
    }
    Ok(lock)
}

/// Waits up to `timeout` until `fd` is readable (for a listening socket: a client is waiting to be accepted). False when the time
/// ran out or a signal interrupted the wait: the caller looks again, so an interruption costs one more pass, never a lost client.
pub(crate) fn wait_readable(fd: &impl AsRawFd, timeout: std::time::Duration) -> io::Result<bool> {
    let mut p = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // Rounded up, so a wait shorter than a millisecond still waits instead of spinning; capped at what poll takes.
    let ms = timeout
        .as_nanos()
        .div_ceil(1_000_000)
        .min(libc::c_int::MAX as u128) as libc::c_int;
    // SAFETY: one valid pollfd that lives through the call.
    let r = unsafe { libc::poll(&mut p, 1, ms) };
    if r < 0 {
        let e = io::Error::last_os_error();
        return if e.kind() == io::ErrorKind::Interrupted {
            Ok(false)
        } else {
            Err(e)
        };
    }
    Ok(r > 0)
}

pub(crate) fn local_connect(path: &std::path::Path) -> io::Result<crate::LocalStream> {
    Ok(crate::LocalStream {
        inner: std::os::unix::net::UnixStream::connect(path)?,
    })
}

pub(crate) fn enable_raw_mode() -> io::Result<crate::RawMode> {
    // SAFETY: an all-zero termios is a valid buffer for tcgetattr to fill.
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: fd 0 is stdin; `saved` is a valid out-pointer.
    cvt(unsafe { libc::tcgetattr(0, &mut saved) })?;
    let mut raw = saved;
    // SAFETY: `raw` is a valid termios.
    unsafe { libc::cfmakeraw(&mut raw) };
    // SAFETY: fd 0 is stdin; `raw` is a valid termios.
    cvt(unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) })?;
    Ok(crate::RawMode { saved })
}

/// What a stop signal puts back: the settings the terminal had before raw mode, and the bytes to write first (see
/// [`restore_terminal_on_stop`]). Read by the signal handler, so both are lock-free: a `OnceLock` read is one atomic load.
static STOP_SAVED: std::sync::OnceLock<libc::termios> = std::sync::OnceLock::new();
static STOP_RESET: std::sync::OnceLock<&'static [u8]> = std::sync::OnceLock::new();

extern "C" fn on_stop_restore(sig: libc::c_int) {
    // Only async-signal-safe calls: `write`, `tcsetattr`, `raise`. The disposition went back to the default when the handler was
    // entered (`SA_RESETHAND`), so the `raise` below, delivered when the handler returns, ends the process by this signal as before.
    if let Some(reset) = STOP_RESET.get() {
        // SAFETY: a valid buffer; fd 1 is stdout; a short or failed write cannot be helped here.
        unsafe { libc::write(1, reset.as_ptr().cast(), reset.len()) };
    }
    if let Some(saved) = STOP_SAVED.get() {
        // SAFETY: restoring settings previously read from fd 0.
        unsafe { libc::tcsetattr(0, libc::TCSANOW, saved) };
    }
    // SAFETY: plain system call.
    unsafe { libc::raise(sig) };
}

/// See [`crate::restore_terminal_on_stop`].
pub(crate) fn restore_terminal_on_stop(
    saved: &libc::termios,
    reset: &'static [u8],
) -> io::Result<()> {
    let _ = STOP_SAVED.set(*saved);
    let _ = STOP_RESET.set(reset);
    for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        // SAFETY: an all-zero sigaction is valid; the handler only calls async-signal-safe functions.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = on_stop_restore as extern "C" fn(libc::c_int) as libc::sighandler_t;
        action.sa_flags = libc::SA_RESETHAND;
        // SAFETY: valid pointers to a filled action; the old action is not wanted.
        unsafe { libc::sigemptyset(&mut action.sa_mask) };
        // SAFETY: as above.
        cvt(unsafe { libc::sigaction(sig, &action, std::ptr::null_mut()) })?;
    }
    Ok(())
}

pub(crate) fn restore_mode(saved: &libc::termios) {
    // SAFETY: restoring settings previously read from the same fd.
    unsafe { libc::tcsetattr(0, libc::TCSANOW, saved) };
}

/// Terminal settings for typing a secret: no echo, no line editing by the terminal and no signals (Ctrl-C arrives as a byte, so the
/// reader can restore the settings before it stops). Restored when dropped, on every path. With `discard_pending`, input that is
/// waiting unread is dropped as the settings change (`TCSAFLUSH`), so what was typed before the prompt never joins the secret.
pub(crate) struct SecretMode(libc::termios);

impl Drop for SecretMode {
    fn drop(&mut self) {
        restore_mode(&self.0);
    }
}

pub(crate) fn secret_mode(discard_pending: bool) -> io::Result<SecretMode> {
    // SAFETY: an all-zero termios is a valid buffer for tcgetattr to fill.
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: fd 0 is stdin; `saved` is a valid out-pointer. Fails (ENOTTY) when stdin is not a terminal.
    if unsafe { libc::tcgetattr(0, &mut saved) } != 0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "stdin is not a terminal",
        ));
    }
    let mut quiet = saved;
    quiet.c_lflag &= !(libc::ECHO | libc::ICANON | libc::ISIG);
    quiet.c_cc[libc::VMIN] = 1;
    quiet.c_cc[libc::VTIME] = 0;
    let when = if discard_pending {
        libc::TCSAFLUSH
    } else {
        libc::TCSANOW
    };
    // SAFETY: fd 0 is stdin; `quiet` is a valid termios.
    cvt(unsafe { libc::tcsetattr(0, when, &quiet) })?;
    Ok(SecretMode(saved))
}

/// The bytes of the keys that arrive on the terminal on stdin within `wait` (`crate::read_confirmation_until`): `None` when none came
/// or a signal interrupted the wait, empty at the end of input. Read from fd 0 directly, never through `std::io::stdin`'s buffer.
pub(crate) fn terminal_key(wait: std::time::Duration) -> io::Result<Option<Vec<u8>>> {
    if !wait_readable(&io::stdin(), wait)? {
        return Ok(None);
    }
    let mut buf = [0u8; 64];
    // SAFETY: fd 0 is stdin; `buf` is valid for writes of its length.
    let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        let e = io::Error::last_os_error();
        return match e.kind() {
            io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock => Ok(None),
            _ => Err(e),
        };
    }
    Ok(Some(buf[..n.unsigned_abs()].to_vec()))
}

pub(crate) fn terminal_size() -> Option<(u16, u16)> {
    // SAFETY: an all-zero winsize is a valid out-buffer.
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: fd 1 is stdout; `ws` is a valid out-pointer.
    let r = unsafe { libc::ioctl(1, libc::TIOCGWINSZ as _, &mut ws) };
    (r == 0 && ws.ws_col > 0 && ws.ws_row > 0).then_some((ws.ws_col, ws.ws_row))
}

pub(crate) fn status(s: std::process::ExitStatus) -> ExitStatus {
    ExitStatus {
        code: s.code(),
        signal: s.signal(),
    }
}

/// Serializes PTY creation. Between `openpty` and the close-on-exec flag, and until the parent closes its
/// copy of the slave, a concurrently spawned process would inherit the descriptors and keep this terminal
/// open forever (its output would never reach end-of-file). Other spawns in this process wait here.
static SPAWN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Marks every open descriptor from `first` up close-on-exec. Async-signal-safe (system calls only): it runs between fork and exec.
pub(crate) fn cloexec_from(first: libc::c_int) {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // CLOSE_RANGE_CLOEXEC (Linux 5.11): the whole range in one call.
        // SAFETY: plain system call with integer arguments.
        let r = unsafe {
            libc::syscall(
                libc::SYS_close_range,
                first as libc::c_uint,
                libc::c_uint::MAX,
                4 as libc::c_uint,
            )
        };
        if r == 0 {
            return;
        }
    }
    for fd in first..descriptor_limit() {
        // SAFETY: plain fcntl; a descriptor that is not open is `EBADF`, which is ignored.
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
}

/// See [`crate::keep_only_standard_streams`].
pub(crate) fn keep_only_standard_streams(cmd: &mut Command) {
    // SAFETY: runs between fork and exec and only makes system calls (getrlimit, fcntl, close_range), which are async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            cloexec_from(3);
            Ok(())
        })
    };
}

/// Closes every descriptor from `first` up, for good. Only where nothing else of this process uses them (its own start).
pub(crate) fn close_from(first: libc::c_int) {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // SAFETY: plain system call with integer arguments.
        let r = unsafe {
            libc::syscall(
                libc::SYS_close_range,
                first as libc::c_uint,
                libc::c_uint::MAX,
                0 as libc::c_uint,
            )
        };
        if r == 0 {
            return;
        }
    }
    for fd in first..descriptor_limit() {
        // SAFETY: plain close; a descriptor that is not open is `EBADF`, which is ignored.
        unsafe { libc::close(fd) };
    }
}

/// One past the highest descriptor number worth trying: the soft limit, at most 65536 (a limit of millions would take seconds).
fn descriptor_limit() -> libc::c_int {
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: a valid out pointer.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE as _, &mut lim) } == 0 {
        lim.rlim_cur.min(65536) as libc::c_int
    } else {
        1024
    }
}

/// The lock that serializes starting programs (see [`SPAWN_LOCK`]), for code that makes descriptors outside a spawn.
#[allow(dead_code)] // used only where `pipe2` is missing
pub(crate) fn spawn_lock() -> std::sync::MutexGuard<'static, ()> {
    SPAWN_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// See [`crate::free_space`].
pub(crate) fn free_space(path: &std::path::Path) -> io::Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: an all-zero `statvfs` is a valid value of the plain C struct the call fills.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: a NUL-terminated path and a struct that outlive the call.
    cvt(unsafe { libc::statvfs(c.as_ptr(), &mut st) })?;
    // The field widths differ between systems (32 bits on some); widened, never truncated.
    #[allow(clippy::unnecessary_cast)]
    let (avail, size) = (st.f_bavail as u64, st.f_frsize as u64);
    Ok(avail.saturating_mul(size))
}

#[cfg(test)]
pub(crate) fn spawn(config: &PtyConfig) -> io::Result<Box<dyn Pty>> {
    spawn_in(config, None)
}

/// See [`crate::spawn`] (no `folder`) and [`crate::spawn_in`].
pub(crate) fn spawn_in(
    config: &PtyConfig,
    folder: Option<&crate::HeldFolder>,
) -> io::Result<Box<dyn Pty>> {
    let (child, writer) = start_child(config, folder, false, false)?;
    start_reader(child, writer)
}

/// Starts `config` on a new PTY in a session of its own and returns the child and the master. `die_with_parent` (Linux only,
/// ADR-0079 decision 8): the child's first process gets `SIGHUP` when the process that started it ends; the kernel ties this to the
/// *thread* that forked, so the caller starts from a thread that lives as long as the process. `clear_env`: the child gets exactly
/// `config.env` (plus a default `TERM`) and nothing of this process's own environment.
pub(crate) fn start_child(
    config: &PtyConfig,
    folder: Option<&crate::HeldFolder>,
    die_with_parent: bool,
    clear_env: bool,
) -> io::Result<(Child, File)> {
    #[cfg(not(target_os = "linux"))]
    let _ = die_with_parent;
    let _guard = SPAWN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut ws = winsize(config.cols, config.rows);
    let (mut master, mut slave) = (-1, -1);
    // SAFETY: the out-pointers are valid; null name/termios are allowed; `ws` outlives the call. The
    // pointer mutability of the last three parameters differs between Linux (`*const`) and macOS (`*mut`),
    // so mutable pointers are passed, which coerce to both.
    #[allow(clippy::unnecessary_mut_passed)] // required by the macOS signature, see above
    let r = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut ws,
        )
    };
    cvt(r)?;
    // SAFETY: openpty returned two fresh descriptors that nothing else owns.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    // Neither side may leak into unrelated processes. The child gets the slave as stdin/stdout/stderr
    // (duplicated by `Command`, which clears the flag on those copies).
    for fd in [&master, &slave] {
        // SAFETY: plain fcntl on a descriptor we own.
        cvt(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) })?;
    }

    let Some((program, args)) = config.argv.split_first() else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "argv is empty"));
    };
    #[cfg(target_os = "macos")]
    let (program, args, git_config) = match &config.sandbox {
        Some(sb) => {
            let a: Vec<std::ffi::OsString> = args.iter().map(Into::into).collect();
            crate::macos::wrap(sb, program.as_ref(), &a)?
        }
        None => (
            program.into(),
            args.iter()
                .map(Into::into)
                .collect::<Vec<std::ffi::OsString>>(),
            None,
        ),
    };
    let mut cmd = Command::new(program);
    cmd.args(args);
    // F1012: in a held folder the child moves into the folder the handle holds (below), never by its path.
    let held = folder.map(|f| f.dir.as_raw_fd());
    if let (Some(cwd), None) = (&config.cwd, held) {
        cmd.current_dir(cwd);
    }
    if clear_env {
        cmd.env_clear();
    } else {
        for (name, _) in std::env::vars_os() {
            if crate::env_removed(&name, &config.env_remove) {
                cmd.env_remove(name);
            }
        }
    }
    if !config.env.iter().any(|(k, _)| k == "TERM") {
        cmd.env("TERM", "xterm-256color");
    }
    cmd.envs(config.env.iter().map(|(k, v)| (k, v)));
    cmd.stdin(Stdio::from(slave.try_clone()?));
    cmd.stdout(Stdio::from(slave.try_clone()?));
    cmd.stderr(Stdio::from(slave));
    let limits = config.limits;
    // Made before the fork: the child only applies it.
    #[cfg(target_os = "linux")]
    let ruleset = config
        .sandbox
        .as_ref()
        .map(|sb| crate::linux::ruleset(sb, folder, config.cwd.as_deref()))
        .transpose()?;
    #[cfg(target_os = "linux")]
    let git_config = ruleset.as_ref().and_then(|r| r.git_config.as_deref());
    #[cfg(target_os = "macos")]
    let git_config = git_config.as_deref();
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Some(file) = git_config {
        cmd.envs(crate::git_config_env(file));
    }
    // SAFETY: runs between fork and exec and only calls async-signal-safe functions (fchdir, fcntl, getrlimit, setsid, ioctl,
    // setrlimit). `held` is a descriptor of `folder`, which the caller keeps open until `spawn` returns.
    unsafe {
        cmd.pre_exec(move || {
            if let Some(fd) = held {
                cvt(libc::fchdir(fd))?;
            }
            // The agent keeps the terminal (0, 1, 2) and nothing else: not what this process was handed at its start (a pipe of
            // its starter, a log), and not a descriptor another thread of this process has open without close-on-exec at this
            // moment. Where a pipe is not made close-on-exec in one step (macOS: std makes it, then marks it), a pipe another thread
            // was making (a `git` whose output the node reads, another start's `exec` report) stayed open in the agent for its whole
            // life, and whoever waited for that pipe's end waited as long (#1011). Marked here, not closed, so std's own pipe that
            // reports a failed `exec` to the parent still works; it all closes at `exec`.
            cloexec_from(3);
            #[cfg(target_os = "linux")]
            if die_with_parent {
                cvt(libc::prctl(
                    libc::PR_SET_PDEATHSIG,
                    libc::SIGHUP as libc::c_ulong,
                ))?;
            }
            // New session so the child leads its own process group, then make the PTY its controlling terminal.
            cvt(libc::setsid())?;
            cvt(libc::ioctl(0, libc::TIOCSCTTY as _, 0))?;
            if let Some(secs) = limits.cpu_seconds {
                set_limit(libc::RLIMIT_CPU as _, secs)?;
            }
            #[cfg(target_os = "linux")]
            if let Some(bytes) = limits.memory_bytes {
                set_limit(libc::RLIMIT_AS as _, bytes)?;
            }
            #[cfg(target_os = "linux")]
            if let Some(r) = &ruleset {
                r.enforce()?;
            }
            Ok(())
        });
    }
    // F1012: checked last, with the confinement already made from the paths, so a folder swapped since it was held is not started in.
    if folder.is_some_and(|f| !f.in_place()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            crate::FolderMoved,
        ));
    }
    let child = cmd.spawn()?;
    drop(cmd); // closes the parent's copies of the slave side, so EOF/EIO reaches the reader on exit

    Ok((child, File::from(master)))
}

/// The consumer-side reader thread of an ordinary [`UnixPty`].
fn start_reader(child: Child, writer: File) -> io::Result<Box<dyn Pty>> {
    let mut read_side = writer.try_clone()?;
    let (tx, output) = sync_channel::<Vec<u8>>(QUEUE);
    let mut child = child;
    let reader = std::thread::Builder::new()
        .name("agora-pty-reader".into())
        .spawn(move || {
            let mut buf = vec![0u8; CHUNK];
            // EIO (Linux) or 0 (macOS) means the slave side closed: the process is gone.
            while let Ok(n) = read_side.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        })
        .inspect_err(|_| {
            // The child is already running: do not leave it behind (or as a zombie) when the reader cannot start.
            let _ = child.kill();
            let _ = child.wait();
        })?;
    Ok(Box::new(UnixPty {
        child,
        writer,
        output,
        reader: Some(reader),
        exited: false,
        reaped: Arc::new(AtomicBool::new(false)),
    }))
}

impl Pty for UnixPty {
    fn output(&self) -> &Receiver<Vec<u8>> {
        &self.output
    }

    fn write_input(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes)
    }

    fn resize(&mut self, cols: u16, rows: u16) -> io::Result<()> {
        let ws = winsize(cols, rows);
        // SAFETY: valid master fd and a pointer to a live winsize.
        cvt(unsafe { libc::ioctl(self.writer.as_raw_fd(), libc::TIOCSWINSZ as _, &ws) }).map(|_| ())
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn kill(&mut self) -> io::Result<()> {
        if self.exited {
            return Ok(());
        }
        kill_group(self.child.id())
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let s = self.child.try_wait()?.map(status);
        if s.is_some() {
            self.exited = true;
            self.reaped.store(true, Ordering::Release);
        }
        Ok(s)
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        let s = status(self.child.wait()?);
        self.exited = true;
        self.reaped.store(true, Ordering::Release);
        Ok(s)
    }

    fn control(&self) -> io::Result<crate::PtyControl> {
        Ok(crate::PtyControl {
            inner: Arc::new(UnixControl {
                input: Mutex::new(self.writer.try_clone()?),
                master: self.writer.try_clone()?,
                pid: self.child.id(),
                reaped: Arc::clone(&self.reaped),
            }),
        })
    }

    fn foreground_pid(&self) -> io::Result<u32> {
        // SAFETY: valid master fd.
        let pgid = cvt(unsafe { libc::tcgetpgrp(self.writer.as_raw_fd()) })?;
        Ok(pgid as u32)
    }
}

impl Drop for UnixPty {
    fn drop(&mut self) {
        let _ = self.kill();
        // Release our ends of the master before waiting. A child blocked writing into a full terminal
        // cannot finish dying (macOS) until the master is drained or closed, so waiting first would hang.
        // Dropping the receiver makes the reader thread exit and close its duplicate of the master.
        self.output = sync_channel(1).1;
        if let Ok(null) = File::open(NULL_DEVICE) {
            drop(std::mem::replace(&mut self.writer, null));
        }
        let _ = self.child.wait();
        self.reaped.store(true, Ordering::Release);
        // Do not join the reader: a descendant that outlived the group could keep the PTY open.
        drop(self.reader.take());
    }
}

/// See [`crate::open_dir`]: `open(O_DIRECTORY)`, close-on-exec as every std open.
pub(crate) fn open_dir(path: &std::path::Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY)
        .open(path)
}

/// See [`crate::visit_dir`]: `fdopendir` on a duplicate of the handle, `readdir`, and `fstatat(AT_SYMLINK_NOFOLLOW)` for each name, so
/// every entry is described from the folder the handle holds and no link is followed.
pub(crate) fn visit_dir(
    dir: &File,
    visitor: &mut dyn FnMut(crate::DirItem) -> std::ops::ControlFlow<()>,
) -> io::Result<()> {
    use std::ffi::{CStr, OsStr};
    use std::os::unix::ffi::OsStrExt;
    // SAFETY: duplicating a live descriptor we own; the copy is owned by the `DIR` below and closed with it.
    let fd = unsafe { libc::fcntl(dir.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` is a fresh descriptor of a directory; on success `fdopendir` takes it over.
    let stream = unsafe { libc::fdopendir(fd) };
    if stream.is_null() {
        let e = io::Error::last_os_error();
        // SAFETY: `fdopendir` failed, so `fd` is still ours to close.
        unsafe { libc::close(fd) };
        return Err(e);
    }
    struct Stream(*mut libc::DIR);
    impl Drop for Stream {
        fn drop(&mut self) {
            // SAFETY: a stream from `fdopendir`, closed once (with its descriptor).
            unsafe { libc::closedir(self.0) };
        }
    }
    let stream = Stream(stream);
    // A duplicate shares its position with the original: start from the first entry whatever was read before.
    // SAFETY: a live stream.
    unsafe { libc::rewinddir(stream.0) };
    loop {
        // `readdir` tells a failure from the end of the folder only through errno, so it is cleared first.
        let checked = clear_errno();
        // SAFETY: a live stream; the entry stays valid until the next `readdir` on it, and is copied before that.
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            let e = io::Error::last_os_error();
            if checked && e.raw_os_error().is_some_and(|code| code != 0) {
                return Err(e);
            }
            break;
        }
        // SAFETY: `d_name` is a NUL-terminated name inside the entry just returned.
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            continue;
        }
        // SAFETY: zeroed `stat` is a valid out-buffer.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: the stream's descriptor, a name from it, a live out-buffer; links are described, not followed.
        let r = unsafe {
            libc::fstatat(
                libc::dirfd(stream.0),
                name.as_ptr(),
                &mut st,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if r != 0 {
            // Gone since `readdir` returned it: like `DirEntry::metadata` failing, it is left out.
            continue;
        }
        let modified = u64::try_from(st.st_mtime)
            .ok()
            .map_or(std::time::UNIX_EPOCH, |s| {
                std::time::UNIX_EPOCH
                    + std::time::Duration::new(s, u32::try_from(st.st_mtime_nsec).unwrap_or(0))
            });
        let item = crate::DirItem {
            name: OsStr::from_bytes(name.to_bytes()).to_os_string(),
            is_dir: st.st_mode & libc::S_IFMT == libc::S_IFDIR,
            len: u64::try_from(st.st_size).unwrap_or(0),
            modified,
        };
        if visitor(item).is_break() {
            break;
        }
    }
    Ok(())
}

/// Sets this thread's errno to 0 (std has no setter); false where that is not known how, and errno must then not be trusted.
fn clear_errno() -> bool {
    #[cfg(target_os = "linux")]
    // SAFETY: the calling thread's own errno slot, always valid.
    unsafe {
        *libc::__errno_location() = 0;
    }
    #[cfg(target_os = "macos")]
    // SAFETY: the calling thread's own errno slot, always valid.
    unsafe {
        *libc::__error() = 0;
    }
    cfg!(any(target_os = "linux", target_os = "macos"))
}

/// Makes `cmd` start in a session of its own (`setsid` in the child before `exec`), detached from this process's terminal: see
/// [`crate::spawn_detached`].
pub(crate) fn detach(cmd: &mut Command) {
    // SAFETY: `setsid` is async-signal-safe and touches no memory of the parent; the child is a fresh fork here.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            // Only standard input, output and error (already redirected) reach the daemon: any other descriptor this process got from
            // its own caller without close-on-exec (a pipe a script reads to its end) is closed at exec, so the caller is never kept
            // waiting. Marked rather than closed, so the runtime's own exec-error pipe still works.
            let max = libc::sysconf(libc::_SC_OPEN_MAX);
            let max = if max <= 0 { 1024 } else { max.min(65_536) } as libc::c_int;
            for fd in 3..max {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags >= 0 && flags & libc::FD_CLOEXEC == 0 {
                    libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
                }
            }
            Ok(())
        });
    }
}

/// The child a caught stop request is passed on to (0: none), and whether one was caught: see [`crate::catch_stop_requests`].
static STOP_TARGET: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
static STOP_CAUGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The first stop request's signal (0: none yet), for [`end_as_stopped`].
static STOP_SIGNAL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// See [`catch_stop_requests_once`]: a second request ends the process at once.
static STOP_ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// When the first stop request came (the monotonic clock, in ms), for [`STOP_ECHO_MS`].
static STOP_AT_MS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// #1119 item 4: with [`catch_stop_requests_once`], a request less than this long after the first is the same stop reaching the process
/// again, not a second one: Ctrl-C on a foreground `agora supervise node` signals the node and its supervisor (one process group), and
/// the supervisor passes its own on; a service manager may signal the whole group as well as the supervisor. A deliberate second request
/// comes later than this.
const STOP_ECHO_MS: i64 = 1000;

/// The monotonic clock in ms (`clock_gettime` is async-signal-safe).
// `time_t` and `c_long` are 32 bits wide on some Unix targets: the conversions are not useless there.
#[allow(clippy::useless_conversion)]
fn monotonic_ms() -> i64 {
    // SAFETY: an all-zero timespec is valid to be written into.
    let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
    // SAFETY: a valid pointer to a timespec; `CLOCK_MONOTONIC` exists on every Unix this builds for.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    i64::from(ts.tv_sec)
        .saturating_mul(1000)
        .saturating_add(i64::from(ts.tv_nsec) / 1_000_000)
}

extern "C" fn on_stop_request(sig: libc::c_int) {
    use std::sync::atomic::Ordering::SeqCst;
    // Only lock-free atomics, `clock_gettime`, `kill`, `signal` and `raise`, all async-signal-safe.
    if STOP_ONCE.load(SeqCst) && STOP_CAUGHT.load(SeqCst) {
        if monotonic_ms().saturating_sub(STOP_AT_MS.load(SeqCst)) < STOP_ECHO_MS {
            // The first request again (see STOP_ECHO_MS): the clean stop goes on.
            return;
        }
        // SAFETY: async-signal-safe calls on this process: the default action of `sig`, then `sig` itself, which ends it.
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
            libc::raise(sig);
        }
        return;
    }
    let _ = STOP_SIGNAL.compare_exchange(0, sig, SeqCst, SeqCst);
    // Before the request is said caught, so a handler that sees it caught also sees when.
    let _ = STOP_AT_MS.compare_exchange(0, monotonic_ms(), SeqCst, SeqCst);
    STOP_CAUGHT.store(true, SeqCst);
    let pid = STOP_TARGET.load(SeqCst);
    if pid > 0 {
        // SAFETY: plain system call on a pid number.
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
}

/// See [`crate::catch_stop_requests_once`].
pub(crate) fn catch_stop_requests_once() -> io::Result<()> {
    STOP_ONCE.store(true, std::sync::atomic::Ordering::SeqCst);
    for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        // A signal ignored on entry (`nohup`, a background job of a shell without job control) stays ignored, as it was before.
        // SAFETY: an all-zero sigaction is valid to be written into.
        let mut old: libc::sigaction = unsafe { std::mem::zeroed() };
        // SAFETY: a null new action only reads the current one into `old`, a valid pointer.
        cvt(unsafe { libc::sigaction(sig, std::ptr::null(), &mut old) })?;
        if old.sa_sigaction == libc::SIG_IGN {
            continue;
        }
        catch_one(sig)?;
    }
    Ok(())
}

/// See [`crate::catch_stop_requests`]. A child started after this gets the default actions back at `exec` (caught signals are reset).
pub(crate) fn catch_stop_requests() -> io::Result<()> {
    for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        catch_one(sig)?;
    }
    Ok(())
}

fn catch_one(sig: libc::c_int) -> io::Result<()> {
    // SAFETY: an all-zero sigaction is valid; the handler only calls async-signal-safe functions.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = on_stop_request as extern "C" fn(libc::c_int) as libc::sighandler_t;
    action.sa_flags = libc::SA_RESTART;
    // SAFETY: valid pointers to a filled action; the old action is not wanted.
    unsafe { libc::sigemptyset(&mut action.sa_mask) };
    // SAFETY: as above.
    cvt(unsafe { libc::sigaction(sig, &action, std::ptr::null_mut()) })?;
    Ok(())
}

/// For [`crate::open_append_owner_created`]: a file these options create is the user's only (0600).
pub(crate) fn owner_only_on_create(options: &mut std::fs::OpenOptions) {
    std::os::unix::fs::OpenOptionsExt::mode(options, 0o600);
}

/// For [`crate::close_to_others`]: clears the group and other bits, keeps the owner's (and the special) bits as they are. A folder
/// others may write in (sticky, or other-writable) is shared on purpose: `InvalidInput`, unchanged.
pub(crate) fn close_to_others(path: &std::path::Path, meta: &std::fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode() & 0o7777;
    if meta.is_dir() && mode & (0o1000 | 0o002) != 0 {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let closed = mode & !0o077;
    if closed == mode {
        return Ok(());
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(closed))
}

/// See [`crate::open_append_private`] (`append`) and [`crate::open_lock_private`] (read and write): the folder is opened as itself (`O_DIRECTORY|O_NOFOLLOW`) and the log is created or opened
/// relative to that descriptor (`openat` with `O_NOFOLLOW`), so neither can be swapped for a link between a check and the open. A new
/// log is created exclusively with mode 0600; an existing one must be a regular file with one name, owned by this user, and is made
/// 0600 through its descriptor when others could read it.
pub(crate) fn open_private(
    dir: &std::path::Path,
    name: &std::ffi::OsStr,
    append: bool,
) -> io::Result<std::fs::File> {
    use std::os::unix::ffi::OsStrExt;
    let cdir = std::ffi::CString::new(dir.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: a NUL-terminated path; the descriptor is owned below.
    let dfd = cvt(unsafe {
        libc::open(
            cdir.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    })
    .map_err(|e| match e.raw_os_error() {
        Some(libc::ELOOP) | Some(libc::ENOTDIR) => {
            io::Error::new(io::ErrorKind::InvalidInput, "a link")
        }
        _ => e,
    })?;
    // SAFETY: a descriptor this call just opened and nothing else owns.
    let dir_fd = unsafe { OwnedFd::from_raw_fd(dfd) };
    open_private_at(&dir_fd, name, append)
}

/// [`open_private`] relative to a folder already held open (`dir`, a directory descriptor): the same checks on `name`, no path looked
/// up (#1000, [`crate::ConfinedDir::open_lock`]).
pub(crate) fn open_private_at(
    dir_fd: &impl AsRawFd,
    name: &std::ffi::OsStr,
    append: bool,
) -> io::Result<std::fs::File> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let refuse = || io::Error::new(io::ErrorKind::InvalidInput, "a link");
    let link_error = |e: io::Error| match e.raw_os_error() {
        Some(libc::ELOOP) | Some(libc::ENOTDIR) => refuse(),
        _ => e,
    };
    let cname = std::ffi::CString::new(name.as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    if name.as_bytes().contains(&b'/') {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let access = if append {
        libc::O_WRONLY | libc::O_APPEND
    } else {
        libc::O_RDWR
    };
    let flags = access | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: a live directory descriptor and a NUL-terminated name.
    let created = unsafe {
        libc::openat(
            dir_fd.as_raw_fd(),
            cname.as_ptr(),
            flags | libc::O_CREAT | libc::O_EXCL,
            0o600 as libc::c_uint,
        )
    };
    if created >= 0 {
        // SAFETY: a descriptor this call just opened.
        return Ok(unsafe { std::fs::File::from_raw_fd(created) });
    }
    let e = io::Error::last_os_error();
    if e.raw_os_error() != Some(libc::EEXIST) {
        return Err(link_error(e));
    }
    // SAFETY: as above.
    // O_NONBLOCK: a FIFO (or a device) planted under the log's name must not hold the open until someone reads it; it is refused below,
    // and a FIFO without a reader is refused at once (ENXIO).
    let fd =
        cvt(unsafe { libc::openat(dir_fd.as_raw_fd(), cname.as_ptr(), flags | libc::O_NONBLOCK) })
            .map_err(|e| match e.raw_os_error() {
                Some(libc::ENXIO) => refuse(),
                _ => link_error(e),
            })?;
    // SAFETY: a descriptor this call just opened.
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let m = file.metadata()?;
    // SAFETY: plain call.
    let me = unsafe { libc::geteuid() };
    if !m.is_file() || m.nlink() > 1 {
        return Err(refuse());
    }
    if m.uid() != me {
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    }
    // A regular file of this user's: writes block as usual again.
    // SAFETY: plain calls on a descriptor `file` owns.
    unsafe {
        let fl = libc::fcntl(file.as_raw_fd(), libc::F_GETFL);
        if fl < 0 || libc::fcntl(file.as_raw_fd(), libc::F_SETFL, fl & !libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    if m.mode() & 0o077 != 0 {
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

/// See [`crate::ensure_private_dir`]: the folder (created by the caller when missing) is opened as itself (`O_DIRECTORY|O_NOFOLLOW`),
/// must be owned by this user, and is made 0700 through that descriptor when others could enter it.
pub(crate) fn ensure_private_dir(dir: &std::path::Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let refuse = || io::Error::new(io::ErrorKind::InvalidInput, "a link");
    let cdir = std::ffi::CString::new(dir.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: a NUL-terminated path; the descriptor is owned below.
    let fd = cvt(unsafe {
        libc::open(
            cdir.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    })
    .map_err(|e| match e.raw_os_error() {
        Some(libc::ELOOP) | Some(libc::ENOTDIR) => refuse(),
        _ => e,
    })?;
    // SAFETY: a descriptor this call just opened and nothing else owns.
    let held = unsafe { OwnedFd::from_raw_fd(fd) };
    // SAFETY: an all-zero stat is a valid out-buffer; a live descriptor.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    cvt(unsafe { libc::fstat(held.as_raw_fd(), &mut st) })?;
    // SAFETY: plain call.
    if st.st_uid != unsafe { libc::geteuid() } {
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    }
    if st.st_mode & 0o077 != 0 {
        // SAFETY: a live descriptor of this user's folder.
        cvt(unsafe { libc::fchmod(held.as_raw_fd(), 0o700) })?;
    }
    Ok(())
}

/// See [`crate::wait_unreaped`].
pub(crate) fn wait_unreaped(pid: u32) -> io::Result<()> {
    let id = libc::id_t::from(pid);
    loop {
        // SAFETY: an all-zero siginfo_t is a valid out-buffer.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: a valid out-pointer; WNOWAIT leaves the child to be reaped by its owner.
        let r = unsafe { libc::waitid(libc::P_PID, id, &mut info, libc::WEXITED | libc::WNOWAIT) };
        if r == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

pub(crate) fn forward_stop_to(pid: Option<u32>) {
    let pid = pid.and_then(|p| i32::try_from(p).ok()).unwrap_or(0);
    STOP_TARGET.store(pid, std::sync::atomic::Ordering::SeqCst);
}

pub(crate) fn stop_requested() -> bool {
    STOP_CAUGHT.load(std::sync::atomic::Ordering::SeqCst)
}

/// See [`crate::end_as_stopped`]: the caught signal (or `SIGTERM`) with its default action again, raised at this process.
pub(crate) fn end_as_stopped() -> ! {
    let sig = match STOP_SIGNAL.load(std::sync::atomic::Ordering::SeqCst) {
        0 => libc::SIGTERM,
        s => s,
    };
    // SAFETY: an all-zero sigaction with `SIG_DFL` is valid; plain system calls on this process.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = libc::SIG_DFL;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(sig, &action, std::ptr::null_mut());
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, sig);
        libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
        libc::raise(sig);
    }
    // Not reached: the default action of each caught signal ends the process. The shell's code for an end by that signal otherwise.
    std::process::exit(128 + sig)
}

/// `SIGTERM` to one process (not its group): see [`crate::terminate_process`].
pub(crate) fn terminate(pid: u32) -> io::Result<()> {
    if pid == 0 {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let pid =
        libc::pid_t::try_from(pid).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: plain system call on a positive pid number.
    cvt(unsafe { libc::kill(pid, libc::SIGTERM) }).map(|_| ())
}

/// Whether a process with this pid exists (one owned by another user counts).
#[allow(dead_code)] // used only on the Unix systems without a start-time reader
pub(crate) fn alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 checks existence and permission only on a positive pid.
    let r = unsafe { libc::kill(pid, 0) };
    r == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// See [`crate::hostname`].
pub(crate) fn hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for `buf.len()` bytes; the result may be unterminated when truncated, handled below.
    if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } != 0 {
        return None;
    }
    let len = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    let name = String::from_utf8_lossy(&buf[..len]).trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// See [`crate::interface_addresses`].
pub(crate) fn interface_addresses() -> io::Result<Vec<crate::InterfaceAddr>> {
    use std::ffi::CStr;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `head` is a valid out pointer; on success the list it receives is released with `freeifaddrs` below, after the last use.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut out = Vec::new();
    let mut cur = head;
    while !cur.is_null() {
        // SAFETY: `cur` is a node of the list `getifaddrs` returned, which stays valid until `freeifaddrs`.
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        if ifa.ifa_addr.is_null() {
            continue;
        }
        // SAFETY: `ifa_addr` is non-null and points to a `sockaddr` whose family says which larger structure it is the start of.
        let family = i32::from(unsafe { (*ifa.ifa_addr).sa_family });
        let ip = if family == libc::AF_INET {
            // SAFETY: the family is AF_INET, so the address is a `sockaddr_in`.
            let a = unsafe { &*ifa.ifa_addr.cast::<libc::sockaddr_in>() };
            IpAddr::V4(Ipv4Addr::from(u32::from_be(a.sin_addr.s_addr)))
        } else if family == libc::AF_INET6 {
            // SAFETY: the family is AF_INET6, so the address is a `sockaddr_in6`.
            let a = unsafe { &*ifa.ifa_addr.cast::<libc::sockaddr_in6>() };
            IpAddr::V6(Ipv6Addr::from(a.sin6_addr.s6_addr))
        } else {
            continue;
        };
        let prefix_len = if ifa.ifa_netmask.is_null() {
            0
        } else {
            netmask_prefix(ifa.ifa_netmask, family)
        };
        let name = if ifa.ifa_name.is_null() {
            String::new()
        } else {
            // SAFETY: `ifa_name` is a NUL-terminated string owned by the list.
            unsafe { CStr::from_ptr(ifa.ifa_name) }
                .to_string_lossy()
                .into_owned()
        };
        let flags = ifa.ifa_flags as libc::c_int;
        out.push(crate::InterfaceAddr {
            name,
            ip,
            prefix_len,
            up: flags & libc::IFF_UP != 0 && flags & libc::IFF_RUNNING != 0,
            loopback: flags & libc::IFF_LOOPBACK != 0,
            transient: false,
        });
    }
    // SAFETY: `head` came from a successful `getifaddrs` and nothing borrowed from the list is used after this point.
    unsafe { libc::freeifaddrs(head) };
    mark_transient(&mut out);
    Ok(out)
}

/// The prefix length of a netmask `sockaddr` without reading past what the system says it holds: the bytes it declares (`sa_len` on
/// macOS and the BSDs, where a netmask can be shorter than a full `sockaddr_in`; the full structure elsewhere) are copied into a zeroed
/// buffer and the address is read from there, so missing bytes count as zero bits.
fn netmask_prefix(mask: *const libc::sockaddr, family: i32) -> u8 {
    let (offset, len) = if family == libc::AF_INET {
        (std::mem::offset_of!(libc::sockaddr_in, sin_addr), 4)
    } else {
        (std::mem::offset_of!(libc::sockaddr_in6, sin6_addr), 16)
    };
    let full = if family == libc::AF_INET {
        std::mem::size_of::<libc::sockaddr_in>()
    } else {
        std::mem::size_of::<libc::sockaddr_in6>()
    };
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
    // SAFETY: `mask` is non-null (checked by the caller) and every `sockaddr` starts with its length byte on these systems.
    let declared = usize::from(unsafe { (*mask).sa_len }).min(full);
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "freebsd")))]
    let declared = full;
    let mut buf = [0u8; 32];
    let n = declared.min(buf.len());
    // SAFETY: `declared` bytes at `mask` belong to the structure the system gave us (the full size where it does not say otherwise);
    // `buf` holds at least `n` bytes and does not overlap.
    unsafe { std::ptr::copy_nonoverlapping(mask.cast::<u8>(), buf.as_mut_ptr(), n) };
    buf.get(offset..offset + len)
        .map_or(0, |b| b.iter().map(|x| x.count_ones()).sum::<u32>() as u8)
}

/// Marks IPv6 addresses the system flags temporary or deprecated. Linux lists them in `/proc/net/if_inet6` (address, index, prefix,
/// scope, flags, name; `IFA_F_TEMPORARY` 0x01, `IFA_F_DEPRECATED` 0x20); other Unix systems are not asked (`getifaddrs` does not carry
/// the flags), so nothing is marked there.
fn mark_transient(list: &mut [crate::InterfaceAddr]) {
    #[cfg(target_os = "linux")]
    {
        let Ok(text) = std::fs::read_to_string("/proc/net/if_inet6") else {
            return;
        };
        for v6 in transient_addrs(&text) {
            let ip = std::net::IpAddr::V6(v6);
            for a in list.iter_mut().filter(|a| a.ip == ip) {
                a.transient = true;
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = list;
}

/// The addresses `/proc/net/if_inet6` flags temporary (0x01) or deprecated (0x20).
#[cfg(target_os = "linux")]
fn transient_addrs(text: &str) -> Vec<std::net::Ipv6Addr> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut f = line.split_whitespace();
        let (Some(addr), Some(_), Some(_), Some(_), Some(flags)) =
            (f.next(), f.next(), f.next(), f.next(), f.next())
        else {
            continue;
        };
        let (Ok(addr), Ok(flags)) = (
            u128::from_str_radix(addr, 16),
            u32::from_str_radix(flags, 16),
        ) else {
            continue;
        };
        if flags & (0x01 | 0x20) != 0 {
            out.push(std::net::Ipv6Addr::from(addr));
        }
    }
    out
}

/// A listener on `127.0.0.1:port` that no other socket can share while it is open (threat model T27). The standard library binds with
/// `SO_REUSEADDR` (so a fixed port can be bound again at once after a run whose connections are still in `TIME_WAIT`; on Linux and
/// macOS that never lets a second socket bind a port that a socket is *listening* on) and without `SO_REUSEPORT`, the option that
/// would let another socket of the same user bind the same address beside it. That is checked here rather than assumed: a listener
/// with `SO_REUSEPORT` on is turned off before it is handed out, and one whose option cannot be read is refused.
pub(crate) fn bind_loopback_exclusive(port: u16) -> io::Result<std::net::TcpListener> {
    let l = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))?;
    if reuse_port(&l)? {
        let off: libc::c_int = 0;
        // SAFETY: a valid socket of this process, an int option of the size given.
        let rc = unsafe {
            libc::setsockopt(
                l.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_REUSEPORT,
                (&off as *const libc::c_int).cast(),
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if rc != 0 || reuse_port(&l)? {
            return Err(io::Error::from(io::ErrorKind::AddrInUse));
        }
    }
    Ok(l)
}

/// Whether `SO_REUSEPORT` is on for `l`.
fn reuse_port(l: &std::net::TcpListener) -> io::Result<bool> {
    let mut v: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: a valid socket of this process; `v` and `len` outlive the call and `len` is the size of `v`.
    let rc = unsafe {
        libc::getsockopt(
            l.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_REUSEPORT,
            (&mut v as *mut libc::c_int).cast(),
            &mut len,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(v != 0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod exclusive_bind_tests {
    use std::os::fd::AsRawFd;

    /// A socket bound with both reuse options on, as a squatter would make it.
    fn squat(port: u16) -> std::io::Result<()> {
        // SAFETY: plain socket calls on a socket this test owns and closes; the option values outlive each call.
        unsafe {
            let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
            assert!(fd >= 0);
            let on: libc::c_int = 1;
            for opt in [libc::SO_REUSEADDR, libc::SO_REUSEPORT] {
                let rc = libc::setsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    opt,
                    (&on as *const libc::c_int).cast(),
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
                assert_eq!(rc, 0);
            }
            let mut sa: libc::sockaddr_in = std::mem::zeroed();
            sa.sin_family = libc::AF_INET as libc::sa_family_t;
            sa.sin_port = port.to_be();
            sa.sin_addr.s_addr = u32::from(std::net::Ipv4Addr::LOCALHOST).to_be();
            let rc = libc::bind(
                fd,
                (&sa as *const libc::sockaddr_in).cast(),
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            );
            let err = std::io::Error::last_os_error();
            let rc = if rc == 0 { libc::listen(fd, 1) } else { rc };
            let err = if rc == 0 { None } else { Some(err) };
            libc::close(fd);
            err.map_or(Ok(()), Err)
        }
    }

    #[test]
    fn the_panel_port_cannot_be_shared_while_it_is_open_and_is_free_again_after() {
        let l = super::bind_loopback_exclusive(0).unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(l.local_addr().unwrap().ip().is_loopback());
        assert!(!super::reuse_port(&l).unwrap());
        // A second socket with SO_REUSEADDR and SO_REUSEPORT cannot bind beside it.
        let e = squat(port).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AddrInUse, "{e}");
        // Neither can a second exclusive bind.
        assert!(super::bind_loopback_exclusive(port).is_err());
        drop(l);
        // Negative control: the same squatter binds once the port is closed, and so does the next run.
        squat(port).unwrap();
        let again = super::bind_loopback_exclusive(port).unwrap();
        assert!(again.as_raw_fd() >= 0);
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)] // expect is allowed in tests
mod tests {
    #[test]
    fn a_log_opened_owner_created_is_the_users_only_and_appends() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        // A fresh folder this test made itself (create_dir fails on one that exists), owner-only, in the shared temp folder.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "agora-owner-created-{}-{nanos}",
            std::process::id()
        ));
        std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700)
            .create(&dir)
            .expect("a fresh folder, private from its creation");
        let log = dir.join("x.log");
        for line in ["a\n", "b\n"] {
            let mut f = crate::open_append_owner_created(&log).expect("open");
            f.write_all(line.as_bytes()).expect("write");
        }
        assert_eq!(std::fs::read_to_string(&log).expect("read"), "a\nb\n");
        let mode = std::fs::metadata(&log).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn closing_to_others_takes_group_and_other_away_and_adds_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("agora-close-{}-{nanos}", std::process::id()));
        std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700)
            .create(&dir)
            .expect("a fresh folder, private from its creation");
        let mode = |p: &std::path::Path| {
            std::fs::symlink_metadata(p)
                .expect("meta")
                .permissions()
                .mode()
                & 0o777
        };
        let set = |p: &std::path::Path, m: u32| {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).expect("chmod");
        };
        let (open, read_only, file) = (dir.join("open"), dir.join("read-only"), dir.join("file"));
        std::fs::create_dir(&open).expect("dir");
        std::fs::create_dir(&read_only).expect("dir");
        std::fs::write(&file, "x").expect("file");
        set(&open, 0o755);
        set(&read_only, 0o555);
        set(&file, 0o644);
        for p in [&open, &read_only, &file] {
            crate::close_to_others(p).expect("close");
        }
        assert_eq!(mode(&open), 0o700);
        assert_eq!(mode(&read_only), 0o500, "the owner is given no write");
        assert_eq!(mode(&file), 0o600);
        // A link is refused and what it points at is left as it is.
        let target = dir.join("target");
        std::fs::create_dir(&target).expect("dir");
        set(&target, 0o755);
        let link = dir.join("link");
        std::os::unix::fs::symlink(&target, &link).expect("link");
        assert_eq!(
            crate::close_to_others(&link).map_err(|e| e.kind()),
            Err(std::io::ErrorKind::InvalidInput)
        );
        assert_eq!(mode(&target), 0o755);
        // A folder shared on purpose (sticky or other-writable, a shared socket folder) is left as it is.
        let shared = dir.join("shared");
        std::fs::create_dir(&shared).expect("dir");
        for m in [0o1777, 0o777] {
            set(&shared, m);
            assert_eq!(
                crate::close_to_others(&shared).map_err(|e| e.kind()),
                Err(std::io::ErrorKind::InvalidInput)
            );
            assert_eq!(
                std::fs::metadata(&shared)
                    .expect("meta")
                    .permissions()
                    .mode()
                    & 0o7777,
                m
            );
        }
        set(&read_only, 0o700);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn if_inet6_lines_flag_temporary_and_deprecated_addresses_only() {
        let text = "\
20010db8000000000000000000000001 02 40 00 80 eth0
20010db8000000000000000000000002 02 40 00 01 eth0
20010db8000000000000000000000003 02 40 00 20 eth0
20010db8000000000000000000000004 02 40 00 81 eth0
fe800000000000000000000000000001 02 40 20 80 eth0
short line
zz10db8000000000000000000000005 02 40 00 01 eth0
20010db8000000000000000000000006 02 40 00 xx eth0
";
        let got = transient_addrs(text);
        let v6 = |s: &str| s.parse::<std::net::Ipv6Addr>().expect("address");
        // 0x01 temporary, 0x20 deprecated, 0x81 (permanent + temporary) all count; 0x80 (permanent) does not; bad lines are skipped.
        assert_eq!(
            got,
            [v6("2001:db8::2"), v6("2001:db8::3"), v6("2001:db8::4")]
        );
    }

    fn v4_mask(octets: [u8; 4], declared_len: Option<u8>) -> libc::sockaddr_in {
        // SAFETY: an all-zero `sockaddr_in` is a valid value.
        let mut m: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        m.sin_family = libc::AF_INET as libc::sa_family_t;
        m.sin_addr.s_addr = u32::from_ne_bytes(octets);
        #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
        {
            m.sin_len = declared_len.unwrap_or(std::mem::size_of::<libc::sockaddr_in>() as u8);
        }
        #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "freebsd")))]
        let _ = declared_len;
        m
    }

    #[test]
    fn a_netmask_becomes_its_prefix_length() {
        let p = |m: &libc::sockaddr_in| {
            netmask_prefix((m as *const libc::sockaddr_in).cast(), libc::AF_INET)
        };
        assert_eq!(p(&v4_mask([255, 255, 255, 0], None)), 24);
        assert_eq!(p(&v4_mask([255, 255, 255, 255], None)), 32);
        assert_eq!(p(&v4_mask([255, 255, 128, 0], None)), 17);
        assert_eq!(p(&v4_mask([0, 0, 0, 0], None)), 0);
        // IPv6.
        // SAFETY: an all-zero `sockaddr_in6` is a valid value.
        let mut m6: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
        m6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
        m6.sin6_addr.s6_addr[..8].fill(0xff);
        #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
        {
            m6.sin6_len = std::mem::size_of::<libc::sockaddr_in6>() as u8;
        }
        assert_eq!(
            netmask_prefix((&m6 as *const libc::sockaddr_in6).cast(), libc::AF_INET6),
            64
        );
    }

    /// Where the system says how long the netmask is (`sa_len`), nothing past that is read: a trimmed mask counts the missing bytes as
    /// zero bits, whatever lies after it in memory.
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
    #[test]
    fn a_trimmed_netmask_is_read_only_as_far_as_it_declares() {
        // Declared length 0: no address bytes at all, so no bits, although the memory holds 255s.
        let m = v4_mask([255, 255, 255, 255], Some(0));
        assert_eq!(
            netmask_prefix((&m as *const libc::sockaddr_in).cast(), libc::AF_INET),
            0
        );
    }

    use std::time::{Duration, Instant};

    use super::*;

    fn run(argv: &[&str]) -> Box<dyn Pty> {
        spawn(&PtyConfig::new(
            argv.iter().map(|s| s.to_string()).collect(),
        ))
        .expect("spawn")
    }

    /// Collect output until it contains `needle` or the timeout passes.
    fn read_until(pty: &dyn Pty, needle: &str) -> String {
        let mut seen = String::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && !seen.contains(needle) {
            if let Ok(chunk) = pty.output().recv_timeout(Duration::from_millis(100)) {
                seen.push_str(&String::from_utf8_lossy(&chunk));
            }
        }
        seen
    }

    #[test]
    fn child_output_arrives_and_exit_code_is_reported() {
        let mut p = run(&["sh", "-c", "echo hello; exit 3"]);
        assert!(read_until(&*p, "hello").contains("hello"));
        assert_eq!(
            p.wait().expect("wait"),
            ExitStatus {
                code: Some(3),
                signal: None
            }
        );
    }

    #[test]
    fn input_reaches_the_child_and_the_tty_echoes_it() {
        let mut p = run(&["cat"]);
        p.write_input(b"ping\n").expect("write");
        let seen = read_until(&*p, "ping\r\nping");
        assert!(seen.contains("ping"), "{seen:?}");
        p.kill().expect("kill");
    }

    #[test]
    fn resize_changes_the_window_size_the_child_sees() {
        let mut p = run(&["sh"]);
        p.resize(100, 30).expect("resize");
        p.write_input(b"stty size\n").expect("write");
        let seen = read_until(&*p, "30 100");
        assert!(seen.contains("30 100"), "{seen:?}");
        p.kill().expect("kill");
    }

    #[test]
    fn initial_size_comes_from_the_config() {
        let mut cfg = PtyConfig::new(vec!["sh".into(), "-c".into(), "stty size".into()]);
        cfg.cols = 91;
        cfg.rows = 17;
        let p = spawn(&cfg).expect("spawn");
        assert!(read_until(&*p, "17 91").contains("17 91"));
    }

    fn sock_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("agora-sock-{}-{name}", std::process::id()))
    }

    #[test]
    fn local_sockets_carry_bytes_both_ways_and_are_owner_only() {
        use std::io::{Read, Write};
        use std::os::unix::fs::PermissionsExt;
        let path = sock_path("rt");
        let listener = crate::local_listen(&path).expect("listen");
        assert_eq!(
            std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777,
            0o600
        );
        let server = std::thread::spawn(move || {
            let mut s = listener.accept().expect("accept");
            let mut buf = [0u8; 5];
            s.read_exact(&mut buf).expect("read");
            s.write_all(&buf.map(|b| b.to_ascii_uppercase()))
                .expect("write");
        });
        let mut c = crate::local_connect(&path).expect("connect");
        c.write_all(b"hello").expect("write");
        let mut out = [0u8; 5];
        c.read_exact(&mut out).expect("read");
        assert_eq!(&out, b"HELLO");
        server.join().expect("server");
        assert!(!path.exists(), "socket file is removed with the listener");
    }

    #[test]
    fn a_stale_socket_file_is_replaced_but_a_live_one_is_not() {
        let path = sock_path("stale");
        // Simulate a crash: bind, then leave the file behind without cleanup.
        let dead = std::os::unix::net::UnixListener::bind(&path).expect("bind");
        drop(dead);
        assert!(path.exists());
        // A PTY test running in parallel can fork before our fd is closed and briefly keep the dead
        // listener alive in its child (until exec), so retry for a moment.
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let live = loop {
            match crate::local_listen(&path) {
                Ok(l) => break l,
                Err(e)
                    if e.kind() == io::ErrorKind::AddrInUse && std::time::Instant::now() < end =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(e) => panic!("stale socket not replaced: {e}"),
            }
        };
        let err = crate::local_listen(&path)
            .err()
            .expect("second listener refused");
        assert_eq!(err.kind(), io::ErrorKind::AddrInUse);
        drop(live);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_live_socket_whose_queue_is_full_is_not_taken_over() {
        // macOS refuses a connection to a full queue at once, as if nobody listened (F202). Linux would block instead.
        let path = sock_path("full");
        let live = crate::local_listen(&path).expect("listen");
        let mut pending = Vec::new();
        loop {
            match std::os::unix::net::UnixStream::connect(&path) {
                Ok(s) if pending.len() < 4096 => pending.push(s),
                Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => break,
                other => panic!("queue never filled: {other:?}"),
            }
        }
        let err = crate::local_listen(&path).err().expect("refused");
        assert_eq!(err.kind(), io::ErrorKind::AddrInUse);
        assert!(path.exists(), "the live server's socket file is kept");
        drop((pending, live));
    }

    #[test]
    fn raw_mode_and_size_need_a_terminal() {
        // Test runners do not give stdin/stdout a terminal; both must fail cleanly rather than misbehave.
        if crate::terminal_size().is_none() {
            assert!(
                crate::RawMode::enable().is_err()
                    || std::io::IsTerminal::is_terminal(&std::io::stdin())
            );
        }
    }

    #[test]
    fn process_limits_are_applied_to_the_child() {
        let mut cfg = PtyConfig::new(vec![
            "sh".into(),
            "-c".into(),
            "ulimit -t; ulimit -v".into(),
        ]);
        cfg.limits = crate::ProcessLimits {
            cpu_seconds: Some(3600),
            memory_bytes: Some(8 << 30),
        };
        let p = spawn(&cfg).expect("spawn");
        let mut out = String::new();
        while let Ok(chunk) = p.output().recv_timeout(std::time::Duration::from_secs(5)) {
            out.push_str(&String::from_utf8_lossy(&chunk));
        }
        let lines: Vec<&str> = out.lines().map(str::trim).collect();
        assert_eq!(lines.first().copied(), Some("3600"), "output: {out:?}");
        #[cfg(target_os = "linux")]
        assert_eq!(lines.get(1).copied(), Some("8388608"), "output: {out:?}"); // KiB
    }

    #[test]
    fn concurrent_spawns_do_not_keep_each_others_terminals_open() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        // A thread spawns long-lived processes while we spawn short-lived ones. Before the fix, a long-lived
        // process inherited a short-lived terminal's slave and its output never ended. They are kept until the end,
        // so a leaked descriptor outlives the wait below. A fixed number: each holds a terminal, and churning through
        // them used to exhaust the machine's terminals on a busy macOS runner (`openpty` then fails with ENXIO).
        // On Linux this catches the leak; on macOS the kernel revokes a terminal when its session leader exits, so a
        // leaked descriptor cannot hold it open and the test only shows that concurrent spawns work.
        const LONG_LIVED: usize = 24;
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let spawner = std::thread::spawn(move || {
            let mut keep = Vec::new();
            while !stop2.load(Ordering::Relaxed) && keep.len() < LONG_LIVED {
                if let Ok(p) = spawn(&PtyConfig::new(vec!["sleep".into(), "30".into()])) {
                    keep.push(p);
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            keep
        });
        for i in 0..40 {
            let p = spawn(&PtyConfig::new(vec![
                "sh".into(),
                "-c".into(),
                "echo hi".into(),
            ]))
            .expect("spawn");
            let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                match p
                    .output()
                    .recv_timeout(std::time::Duration::from_millis(100))
                {
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                        if std::time::Instant::now() > end =>
                    {
                        panic!("terminal {i} never reached end-of-file: its descriptors leaked")
                    }
                    _ => {}
                }
            }
        }
        stop.store(true, Ordering::Relaxed);
        drop(spawner.join().expect("spawner"));
    }

    #[test]
    fn control_works_from_another_thread_while_output_is_read_elsewhere() {
        let mut p = spawn(&PtyConfig::new(vec![
            "sh".into(),
            "-c".into(),
            "stty size; cat".into(),
        ]))
        .expect("spawn");
        let control = p.control().expect("control");
        assert_eq!(control.pid(), p.pid());
        // Input, resize and kill all go through the control while `p` is only read.
        control.resize(97, 31).expect("resize");
        control.write_input(b"ping\n").expect("write");
        let mut out = String::new();
        let end = Instant::now() + Duration::from_secs(5);
        while !out.contains("ping") && Instant::now() < end {
            if let Ok(c) = p.output().recv_timeout(Duration::from_millis(100)) {
                out.push_str(&String::from_utf8_lossy(&c));
            }
        }
        assert!(out.contains("ping"), "{out:?}");
        assert!(control.foreground_pid().is_ok());
        control.kill().expect("kill");
        p.wait().expect("wait");
        // After the child is reaped, a late kill is a harmless no-op (it must not signal a recycled pid).
        control.kill().expect("late kill");
    }

    #[test]
    fn dropping_a_pty_whose_output_nobody_reads_does_not_hang() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let p = spawn(&PtyConfig::new(vec![
                "sh".into(),
                "-c".into(),
                "yes".into(),
            ]))
            .expect("spawn");
            // Let the child fill the terminal buffer and the output queue so it blocks on write.
            std::thread::sleep(std::time::Duration::from_millis(500));
            drop(p);
            let _ = tx.send(());
        });
        assert!(
            rx.recv_timeout(std::time::Duration::from_secs(10)).is_ok(),
            "drop hung with a child blocked on write"
        );
    }

    #[test]
    fn kill_takes_down_the_whole_process_group_quickly() {
        // sh starts a background sleep in the same group; killing the group must end both.
        let mut p = run(&["sh", "-c", "sleep 60 & wait"]);
        std::thread::sleep(Duration::from_millis(200));
        let started = Instant::now();
        p.kill().expect("kill");
        let status = p.wait().expect("wait");
        assert_eq!(status.signal, Some(libc::SIGKILL));
        assert!(started.elapsed() < Duration::from_secs(5));
        // The output channel closes once every holder of the PTY is gone.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match p.output().recv_timeout(Duration::from_millis(100)) {
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                _ if Instant::now() > deadline => panic!("output channel never closed"),
                _ => {}
            }
        }
    }

    /// A descriptor of this process left without close-on-exec, as std's pipes are on macOS between making them and marking them
    /// (#1011): a number high enough that the shell below opens nothing there itself. Closed when the result is dropped.
    fn left_open() -> (File, OwnedFd) {
        let file = File::open("/dev/null").expect("open");
        // SAFETY: plain fcntl on a descriptor we own; F_DUPFD makes a copy without close-on-exec.
        let fd = cvt(unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD, 100) }).expect("dup");
        // SAFETY: fcntl just returned this fresh descriptor, which nothing else owns.
        (file, unsafe { OwnedFd::from_raw_fd(fd) })
    }

    /// A shell command that says whether descriptor `fd` reached it.
    fn says_whether_it_got(fd: &OwnedFd) -> Vec<String> {
        let n = fd.as_raw_fd();
        vec![
            "sh".into(),
            "-c".into(),
            format!("if [ -e /dev/fd/{n} ]; then echo GOT-IT; else echo NOT-GOT; fi"),
        ]
    }

    #[test]
    fn a_terminal_agent_never_gets_a_descriptor_another_thread_left_open() {
        // #1011: the agent kept such a pipe for its whole life, so whoever waited for the pipe's end (another start's `exec` report,
        // a `git` whose output was read) waited as long.
        let (_file, fd) = left_open();
        let p = spawn(&PtyConfig::new(says_whether_it_got(&fd))).expect("spawn");
        let out = read_until(&*p, "GOT");
        assert!(out.contains("NOT-GOT"), "output: {out:?}");
    }

    #[test]
    fn a_child_kept_to_its_standard_streams_never_gets_a_descriptor_another_thread_left_open() {
        // #1011: an adapter agent, started with std's `Command`, kept one the same way.
        let (_file, fd) = left_open();
        let argv = says_whether_it_got(&fd);
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        let out = crate::keep_only_standard_streams(&mut cmd)
            .output()
            .expect("run");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "NOT-GOT");
        // Without it the same command does get it, so the check above proves something.
        let out = Command::new(&argv[0])
            .args(&argv[1..])
            .output()
            .expect("run");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "GOT-IT");
    }

    #[test]
    fn try_wait_is_none_while_running_and_cwd_and_env_apply() {
        let mut cfg = PtyConfig::new(vec![
            "sh".into(),
            "-c".into(),
            "pwd; echo $AGORA_X; sleep 0.3".into(),
        ]);
        cfg.cwd = Some(std::env::temp_dir());
        cfg.env.push(("AGORA_X".into(), "yes".into()));
        let mut p = spawn(&cfg).expect("spawn");
        assert_eq!(p.try_wait().expect("try_wait"), None);
        let seen = read_until(&*p, "yes");
        assert!(seen.contains("yes"), "{seen:?}");
        assert!(p.wait().expect("wait").success());
        assert!(p.try_wait().expect("try_wait").is_some());
    }

    #[test]
    fn inherited_variables_named_in_env_remove_are_left_out_and_set_ones_kept() {
        // A name no other test uses: set for this process, so every child inherits it unless it is removed.
        std::env::set_var("AGORA_TEST_DROP_ME_1", "inherited");
        std::env::set_var("AGORA_TEST_KEEP_ME", "kept");
        let mut cfg = PtyConfig::new(vec![
            "sh".into(),
            "-c".into(),
            "echo \"[${AGORA_TEST_DROP_ME_1:-gone}|$AGORA_TEST_KEEP_ME|$AGORA_TEST_DROP_SET]\"; sleep 0.2".into(),
        ]);
        cfg.env_remove = vec!["AGORA_TEST_DROP_*".into()];
        cfg.env.push(("AGORA_TEST_DROP_SET".into(), "set".into()));
        let mut p = spawn(&cfg).expect("spawn");
        let seen = read_until(&*p, "]");
        assert!(seen.contains("[gone|kept|set]"), "{seen:?}");
        let _ = p.wait();
    }

    /// F1012: the held folder decides where the process starts (moved into through its handle, whatever `cwd` says), a folder
    /// swapped for a link after it was held is refused rather than followed, and a folder is never held through a link.
    #[test]
    fn a_held_folder_is_where_the_process_starts_and_a_swapped_one_is_refused() {
        let base = std::env::temp_dir().join(format!("agora-plat-held-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("io");
        let base = std::fs::canonicalize(&base).expect("io");
        let (held_dir, elsewhere) = (base.join("held"), base.join("elsewhere"));
        std::fs::create_dir_all(&held_dir).expect("io");
        std::fs::create_dir_all(&elsewhere).expect("io");
        let pwd = |folder: &crate::HeldFolder| {
            let mut cfg = PtyConfig::new(vec!["sh".into(), "-c".into(), "pwd -P".into()]);
            cfg.cwd = Some(elsewhere.clone());
            crate::spawn_in(&cfg, folder).map(|mut p| {
                let seen = read_until(&*p, &held_dir.display().to_string());
                let _ = p.wait();
                seen
            })
        };
        let folder = crate::HeldFolder::open(&held_dir).expect("held");
        let seen = pwd(&folder).expect("spawn_in");
        assert!(seen.contains(&held_dir.display().to_string()), "{seen:?}");
        std::fs::rename(&held_dir, base.join("held.old")).expect("io");
        std::os::unix::fs::symlink(&elsewhere, &held_dir).expect("io");
        let refused = pwd(&folder)
            .map(|_| ())
            .expect_err("a swapped folder is not started in");
        assert!(crate::folder_moved(&refused), "{refused:?}");
        let through_link = crate::HeldFolder::open(&held_dir).map(|_| ());
        assert!(
            matches!(&through_link, Err(e) if crate::folder_moved(e)),
            "{through_link:?}"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn spawning_a_missing_program_is_an_error() {
        let err = spawn(&PtyConfig::new(vec!["/definitely/not/a/program".into()]));
        assert!(err.is_err());
        assert!(spawn(&PtyConfig::new(vec![])).is_err());
    }

    #[test]
    fn a_socket_path_that_is_a_regular_file_is_refused_and_the_file_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("agora-plat-sock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("io");
        let file = dir.join("precious");
        std::fs::write(&file, "keep me").expect("io");
        assert!(crate::local_listen(&file).is_err());
        assert_eq!(std::fs::read_to_string(&file).expect("io"), "keep me");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn write_private_replaces_the_file_whole_keeps_it_private_and_leaves_no_temporary_behind() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("agora-plat-priv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("io");
        let f = dir.join("key");
        crate::write_private(&f, b"one").expect("io");
        crate::write_private(&f, b"two-longer").expect("io");
        assert_eq!(std::fs::read(&f).expect("io"), b"two-longer");
        assert_eq!(
            std::fs::metadata(&f).expect("io").permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(std::fs::read_dir(&dir).expect("io").count(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod identity_tests {
    use crate::{kill_process_if_same, process_identity, process_still_running};

    #[test]
    fn a_process_is_known_by_its_start_and_a_reused_pid_is_never_killed() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let id = process_identity(child.id()).unwrap();
        assert!(process_still_running(&id));
        // The same pid with another start time is another process: left alone.
        // Another start time, another boot, another program, or no program named: another process, left alone.
        let others = [
            crate::ProcessIdentity {
                started: id.started + 1,
                ..id.clone()
            },
            crate::ProcessIdentity {
                boot: format!("{}x", id.boot),
                ..id.clone()
            },
            crate::ProcessIdentity {
                name: "victim".into(),
                ..id.clone()
            },
            crate::ProcessIdentity {
                name: String::new(),
                ..id.clone()
            },
        ];
        for other in &others {
            assert!(!process_still_running(other), "{other:?}");
            assert!(!kill_process_if_same(other).unwrap(), "{other:?}");
        }
        assert!(child.try_wait().unwrap().is_none());
        // Its own group is the test's: only the process itself is ended.
        assert!(kill_process_if_same(&id).unwrap());
        assert!(!child.wait().unwrap().success());
        assert!(!process_still_running(&id));
    }
}
