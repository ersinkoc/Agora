//! PTY on Unix (Linux and macOS) via `openpty`.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver};
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

fn status(s: std::process::ExitStatus) -> ExitStatus {
    ExitStatus {
        code: s.code(),
        signal: s.signal(),
    }
}

pub(crate) fn spawn(config: &PtyConfig) -> io::Result<Box<dyn Pty>> {
    let mut ws = winsize(config.cols, config.rows);
    let (mut master, mut slave) = (-1, -1);
    // SAFETY: the out-pointers are valid; null name/termios are allowed; `ws` outlives the call. The
    // pointer mutability of the last three parameters differs between Linux (`*const`) and macOS (`*mut`),
    // so mutable pointers are passed, which coerce to both.
    cvt(unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut ws,
        )
    })?;
    // SAFETY: openpty returned two fresh descriptors that nothing else owns.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    // The child must not inherit the master side.
    // SAFETY: plain fcntl on a descriptor we own.
    cvt(unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) })?;

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
    // SAFETY: runs between fork and exec and only calls async-signal-safe functions (setsid, ioctl).
    unsafe {
        cmd.pre_exec(|| {
            // New session so the child leads its own process group, then make the PTY its controlling terminal.
            cvt(libc::setsid())?;
            cvt(libc::ioctl(0, libc::TIOCSCTTY as _, 0))?;
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
        let pgid = self.child.id() as libc::pid_t;
        // SAFETY: signalling a process group we created with setsid.
        let r = unsafe { libc::killpg(pgid, libc::SIGKILL) };
        if r < 0 {
            let e = io::Error::last_os_error();
            // The group is already gone: nothing left to kill.
            if e.raw_os_error() != Some(libc::ESRCH) {
                return Err(e);
            }
        }
        Ok(())
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let s = self.child.try_wait()?.map(status);
        self.exited |= s.is_some();
        Ok(s)
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        let s = status(self.child.wait()?);
        self.exited = true;
        Ok(s)
    }
}

impl Drop for UnixPty {
    fn drop(&mut self) {
        let _ = self.kill();
        let _ = self.child.wait();
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
