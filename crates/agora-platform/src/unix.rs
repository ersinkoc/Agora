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

fn winsize(cols: u16, rows: u16) -> libc::winsize {
    libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

fn cvt(r: libc::c_int) -> io::Result<libc::c_int> {
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
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    // A socket file nobody serves is left over from a crash: replace it. A live one is an error.
    if path.exists() {
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
    })
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

pub(crate) fn restore_mode(saved: &libc::termios) {
    // SAFETY: restoring settings previously read from the same fd.
    unsafe { libc::tcsetattr(0, libc::TCSANOW, saved) };
}

pub(crate) fn terminal_size() -> Option<(u16, u16)> {
    // SAFETY: an all-zero winsize is a valid out-buffer.
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: fd 1 is stdout; `ws` is a valid out-pointer.
    let r = unsafe { libc::ioctl(1, libc::TIOCGWINSZ as _, &mut ws) };
    (r == 0 && ws.ws_col > 0 && ws.ws_row > 0).then_some((ws.ws_col, ws.ws_row))
}

fn status(s: std::process::ExitStatus) -> ExitStatus {
    ExitStatus {
        code: s.code(),
        signal: s.signal(),
    }
}

/// Serializes PTY creation. Between `openpty` and the close-on-exec flag, and until the parent closes its
/// copy of the slave, a concurrently spawned process would inherit the descriptors and keep this terminal
/// open forever (its output would never reach end-of-file). Other spawns in this process wait here.
static SPAWN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn spawn(config: &PtyConfig) -> io::Result<Box<dyn Pty>> {
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
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(cwd) = &config.cwd {
        cmd.current_dir(cwd);
    }
    if !config.env.iter().any(|(k, _)| k == "TERM") {
        cmd.env("TERM", "xterm-256color");
    }
    cmd.envs(config.env.iter().map(|(k, v)| (k, v)));
    cmd.stdin(Stdio::from(slave.try_clone()?));
    cmd.stdout(Stdio::from(slave.try_clone()?));
    cmd.stderr(Stdio::from(slave));
    let limits = config.limits;
    // SAFETY: runs between fork and exec and only calls async-signal-safe functions (setsid, ioctl,
    // setrlimit).
    unsafe {
        cmd.pre_exec(move || {
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
            Ok(())
        });
    }
    let child = cmd.spawn()?;
    drop(cmd); // closes the parent's copies of the slave side, so EOF/EIO reaches the reader on exit

    let writer = File::from(master);
    let mut read_side = writer.try_clone()?;
    let (tx, output) = sync_channel::<Vec<u8>>(QUEUE);
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
        if let Ok(null) = File::open("/dev/null") {
            drop(std::mem::replace(&mut self.writer, null));
        }
        let _ = self.child.wait();
        self.reaped.store(true, Ordering::Release);
        // Do not join the reader: a descendant that outlived the group could keep the PTY open.
        drop(self.reader.take());
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)] // expect is allowed in tests
mod tests {
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
        // A thread keeps spawning long-lived processes while we spawn short-lived ones. Before the fix, a
        // long-lived process inherited a short-lived terminal's slave and its output never ended.
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let spawner = std::thread::spawn(move || {
            let mut keep = Vec::new();
            while !stop2.load(Ordering::Relaxed) {
                if let Ok(p) = spawn(&PtyConfig::new(vec!["sleep".into(), "30".into()])) {
                    keep.push(p);
                }
                if keep.len() > 60 {
                    keep.clear();
                }
            }
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
        spawner.join().expect("spawner");
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
    fn spawning_a_missing_program_is_an_error() {
        let err = spawn(&PtyConfig::new(vec!["/definitely/not/a/program".into()]));
        assert!(err.is_err());
        assert!(spawn(&PtyConfig::new(vec![])).is_err());
    }
}
