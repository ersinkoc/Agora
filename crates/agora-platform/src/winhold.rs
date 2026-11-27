//! ADR-0079 W1: the Windows primitives of the terminal holder, the process that owns the agents' pseudo-consoles and job so they
//! outlive their node. The holder itself (`agora hold`), its link to the node and the attach protocol are W2; here are only the parts
//! that are Windows' own, each tested by itself:
//! - starting the holder out of the supervisor's job, and never inside a job that forbids leaving it ([`start_holder`]);
//! - the holder's terminal, read only while no node is attached ([`crate::conpty::HeldTerminal`]);
//! - handing a terminal's pipe ends to the node: the node's process is opened once, checked by its start time on that same handle, and
//!   the handles are duplicated into that handle ([`OtherProcess`]), so the check and the use cannot name two processes;
//! - the pid of the client on the holder's pipe ([`crate::windows::Stream::client_pid`]);
//! - watching the node's process end ([`OtherProcess::ended`]).
#![allow(dead_code)] // ADR-0079 W2 calls these; until then only the tests do.

use std::io;
use std::path::Path;
use std::ptr::null_mut;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, DuplicateHandle, DUPLICATE_CLOSE_SOURCE, DUPLICATE_SAME_ACCESS, FILETIME, HANDLE,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, OpenProcess, WaitForSingleObject, PROCESS_DUP_HANDLE,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
};

/// ADR-0079 decision 4: what the holder keeps of one terminal's output while no node is attached.
pub(crate) const HELD_BUFFER: usize = 1 << 20;

/// ADR-0079 decision 2: starts the holder detached from this process, its console and its job. When the job forbids leaving it (a
/// Task Scheduler task's job), nothing is started and the error is `PermissionDenied`: inside the supervisor's kill-on-close job the
/// holder would die with the supervisor, which is what it exists to survive. Standard output and error go to `log`. Returns its pid.
pub(crate) fn start_holder(cmd: &std::process::Command, log: &Path) -> io::Result<u32> {
    let out = crate::open_append_private(log)?;
    crate::winproc::spawn_detached_strict(cmd, &out).map(|d| d.pid)
}

/// Another process (the node), opened once for everything the holder does with it.
pub(crate) struct OtherProcess {
    handle: HANDLE,
    pid: u32,
}

// SAFETY: a process handle is an opaque id any thread may use.
unsafe impl Send for OtherProcess {}
unsafe impl Sync for OtherProcess {}

impl OtherProcess {
    /// Opens process `pid` and checks, on the handle just opened, that it started at `started` (the value of
    /// [`crate::process_start_time`]). `NotFound` when no such process runs, or another one has its pid now.
    pub(crate) fn open(pid: u32, started: u64) -> io::Result<Self> {
        // SAFETY: plain call; the handle is owned by the value below and closed on drop.
        let handle = unsafe {
            OpenProcess(
                PROCESS_DUP_HANDLE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        if handle.is_null() {
            let e = io::Error::last_os_error();
            // ERROR_INVALID_PARAMETER: no process has this pid (any more), which is `NotFound` as on Unix.
            if e.raw_os_error() == Some(87) {
                return Err(io::Error::from(io::ErrorKind::NotFound));
            }
            return Err(e);
        }
        let p = Self { handle, pid };
        // An ended process stays openable, start time and all, while any handle to it is open (another watch's, for one): it does not
        // run any more, so it is `NotFound` as on Unix, where an ended process is gone once reaped.
        if p.start_time()? != started || p.ended(Duration::ZERO) {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        Ok(p)
    }

    pub(crate) fn pid(&self) -> u32 {
        self.pid
    }

    /// When this process started (100 ns units since 1601), read from the handle itself.
    pub(crate) fn start_time(&self) -> io::Result<u64> {
        // SAFETY: all-zero FILETIMEs are valid out-buffers.
        let (mut created, mut exited, mut kernel, mut user): (
            FILETIME,
            FILETIME,
            FILETIME,
            FILETIME,
        ) = unsafe { std::mem::zeroed() };
        // SAFETY: a live handle with PROCESS_QUERY_LIMITED_INFORMATION and valid out pointers.
        if unsafe {
            GetProcessTimes(
                self.handle,
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }

    /// Duplicates `handle` (of this process) into the other process, with the same access, not inheritable. Returns the value the
    /// other process uses; it owns that handle from now on.
    pub(crate) fn duplicate_into(&self, handle: HANDLE) -> io::Result<usize> {
        let mut theirs: HANDLE = null_mut();
        // SAFETY: `handle` is live in this process; `self.handle` has PROCESS_DUP_HANDLE; valid out pointer.
        let ok = unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                handle,
                self.handle,
                &mut theirs,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(theirs as usize)
    }

    /// Closes, in the other process, a handle this process duplicated into it ([`Self::duplicate_into`]) and that it never got to use
    /// (the frame naming it was not delivered): `DUPLICATE_CLOSE_SOURCE` with no target closes it there.
    pub(crate) fn close_theirs(&self, value: usize) -> io::Result<()> {
        // SAFETY: `self.handle` has PROCESS_DUP_HANDLE; with DUPLICATE_CLOSE_SOURCE and no target, the source handle (in the other
        // process) is only closed.
        let ok = unsafe {
            DuplicateHandle(
                self.handle,
                value as HANDLE,
                null_mut(),
                null_mut(),
                0,
                0,
                DUPLICATE_CLOSE_SOURCE,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Whether the process has ended, waiting at most `wait` for it (ADR-0079 decision 4: the holder starts reading when it has).
    /// A failed wait (`WAIT_FAILED`, which an open handle with `SYNCHRONIZE` does not give) counts as not ended: the process is still
    /// watched, and a watcher that wrongly took it for ended would start a second reader beside the node's.
    pub(crate) fn ended(&self, wait: Duration) -> bool {
        let ms = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX - 1);
        // SAFETY: a live handle with SYNCHRONIZE.
        unsafe { WaitForSingleObject(self.handle, ms) == WAIT_OBJECT_0 }
    }
}

impl Drop for OtherProcess {
    fn drop(&mut self) {
        // SAFETY: owned and closed once.
        unsafe { CloseHandle(self.handle) };
    }
}

// ----- M2.6: a program on pipes (ADR-0093 step 3) -----

/// A program the holder runs on a pair of pipes instead of a pseudo-console (a structured agent: an adapter or an ACP bridge): its input,
/// its output, its process (so its exit status) and its own kill-on-close job, so everything it starts ends with it, and with the holder
/// (ADR-0079 decision 8). Started with `CREATE_NO_WINDOW`: a console program on pipes never opens a window. Its stderr is the holder's
/// own (the holder's log).
#[derive(Debug)]
pub struct HeldProcess {
    child: std::process::Child,
    job: JobHandle,
    io: Option<(std::process::ChildStdin, std::process::ChildStdout)>,
    exit: Option<crate::ExitStatus>,
}

/// An owned job object handle.
#[derive(Debug)]
struct JobHandle(HANDLE);

// SAFETY: a job handle is an opaque id any thread may use.
unsafe impl Send for JobHandle {}
unsafe impl Sync for JobHandle {}

impl Drop for JobHandle {
    fn drop(&mut self) {
        // SAFETY: owned and closed once; closing the last handle of a kill-on-close job ends what is left in it.
        unsafe { CloseHandle(self.0) };
    }
}

/// A kill-on-close job with nothing else set, with `process` in it.
fn kill_on_close_job(process: HANDLE) -> io::Result<JobHandle> {
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    // SAFETY: plain call.
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        return Err(io::Error::last_os_error());
    }
    let job = JobHandle(job);
    // SAFETY: an all-zero limit structure means "no limits"; only kill-on-close is set.
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: `info` is the structure the class names, with its size; then two live handles.
    let ok = unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) != 0
            && AssignProcessToJobObject(job.0, process) != 0
    };
    if !ok {
        return Err(io::Error::last_os_error());
    }
    Ok(job)
}

impl HeldProcess {
    /// Starts `argv` with exactly `env` (the holder's own environment is a stale snapshot of whoever started it and is never passed
    /// on), in `cwd` when given. A bare name installed as a `.cmd` shim (`npx`) is found on `env`'s `PATH` as the terminal start does;
    /// a batch file is run as the standard library runs one (its arguments quoted for `cmd.exe`, CVE-2024-24576).
    pub fn start(
        argv: &[String],
        cwd: Option<&std::path::Path>,
        env: &[(String, String)],
    ) -> io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        let Some((program, args)) = argv.split_first() else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "argv is empty"));
        };
        let path = env
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
            .map(|(_, p)| std::ffi::OsStr::new(p.as_str()));
        let mut cmd = std::process::Command::new(crate::windows::program_path(program, path));
        cmd.args(args);
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        cmd.env_clear();
        cmd.envs(env.iter().map(|(k, v)| (k, v)));
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .creation_flags(CREATE_NO_WINDOW);
        let mut child = cmd.spawn()?;
        let job = match kill_on_close_job(child.as_raw_handle().cast()) {
            Ok(j) => j,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let Some(io) = child.stdin.take().zip(child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("pipes missing"));
        };
        Ok(Self {
            child,
            job,
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

    /// What a signal is for an agent on pipes on Windows: it has no console to raise Ctrl-C or a close event in, so `Terminate`, `Hangup`
    /// and `Kill` end its whole job, and `Interrupt` does nothing (a structured agent is interrupted through its own protocol, as the
    /// node does with ACP's cancel). Nothing once its end was collected.
    pub fn signal(&self, signal: crate::held::HeldSignal) -> io::Result<()> {
        use crate::held::HeldSignal;
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        if self.exit.is_some() {
            return Ok(());
        }
        match signal {
            HeldSignal::Interrupt => Ok(()),
            HeldSignal::Terminate | HeldSignal::Hangup | HeldSignal::Kill => {
                // SAFETY: a live job handle.
                if unsafe { TerminateJobObject(self.job.0, 1) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            }
        }
    }

    /// The agent's exit status once it ended, kept from then on. When it has ended, whatever it left in its job is ended with it: an
    /// adapter that ended or crashed must not leave processes the holder cannot name, which would also hold its output open for good.
    pub fn try_wait(&mut self) -> io::Result<Option<crate::ExitStatus>> {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        if self.exit.is_none() {
            if let Some(s) = self.child.try_wait()? {
                // SAFETY: a live job handle; its leader has ended, so only what it left behind is ended here.
                unsafe { TerminateJobObject(self.job.0, 1) };
                self.exit = Some(crate::ExitStatus {
                    code: s.code(),
                    signal: None,
                });
            }
        }
        Ok(self.exit)
    }

    /// Ends the agent's whole job and waits for its process, so nothing is left of it.
    pub fn kill_and_reap(&mut self) {
        let _ = self.signal(crate::held::HeldSignal::Kill);
        let code = self.child.wait().ok().and_then(|s| s.code());
        self.exit = Some(crate::ExitStatus { code, signal: None });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};
    use std::time::Instant;

    use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    use super::*;
    use crate::conpty::HeldTerminal;
    use crate::PtyConfig;

    const ROLE: &str = "AGORA_W1_TEST_ROLE";

    /// This test binary again, running only the helper `name`.
    fn helper(name: &str, role: &str) -> Command {
        let mut c = Command::new(std::env::current_exe().unwrap());
        c.args(["--exact", name, "--nocapture", "--test-threads=1"])
            .env(ROLE, role);
        c
    }

    fn in_job(pid: u32, job: HANDLE) -> bool {
        // SAFETY: plain calls; the process handle is closed right after.
        unsafe {
            let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            assert!(!p.is_null(), "process {pid} is gone");
            let mut inside = 0;
            IsProcessInJob(p, job, &mut inside);
            CloseHandle(p);
            inside != 0
        }
    }

    /// Not a test: run by `the_holder_leaves_a_job_that_lets_it_and_never_starts_inside_one_that_does_not` in a fresh process. Puts
    /// itself in a kill-on-close job (that allows breakaway or not, by its role), then starts a long `ping` the strict way and the old
    /// way, and prints what happened.
    #[test]
    fn job_helper() {
        let Ok(role) = std::env::var(ROLE) else {
            return;
        };
        let breakaway = match role.as_str() {
            "breakaway" => true,
            "locked" => false,
            _ => return,
        };
        // SAFETY: plain calls on a job this process owns.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        assert!(!job.is_null());
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | if breakaway {
                JOB_OBJECT_LIMIT_BREAKAWAY_OK
            } else {
                0
            };
        unsafe {
            assert!(
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&raw const info).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) != 0
            );
            assert!(AssignProcessToJobObject(job, GetCurrentProcess()) != 0);
        }
        let log = std::env::temp_dir().join(format!("agora-w1-job-{}.log", std::process::id()));
        // A full path: the detached start passes the program as the application name, which is not searched for.
        let system = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap());
        let mut ping = Command::new(system.join("System32").join("PING.EXE"));
        ping.args(["-n", "30", "127.0.0.1"]);
        match start_holder(&ping, &log) {
            Ok(pid) => {
                println!("STRICT ok injob={}", in_job(pid, job));
                let _ = crate::winproc::terminate(pid);
            }
            Err(e) => println!("STRICT err {:?}", e.kind()),
        }
        let out = crate::open_append_private(&log).unwrap();
        let old = crate::winproc::spawn_detached(&ping, &out).unwrap();
        println!("OLD in_job={} injob={}", old.in_job, in_job(old.pid, job));
        let _ = crate::winproc::terminate(old.pid);
        drop(out);
        let _ = std::fs::remove_file(&log);
    }

    fn job_report(role: &str) -> (String, String) {
        let out = helper("winhold::tests::job_helper", role).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let line = |tag: &str| {
            text.lines()
                .find_map(|l| l.find(tag).map(|at| l[at..].to_string()))
                .unwrap_or_else(|| panic!("no {tag} line: {text}"))
        };
        (line("STRICT "), line("OLD "))
    }

    /// ADR-0079 decision 2 as measured on a real machine: in a job that allows breakaway (the supervisor's) the holder leaves it; in
    /// one that forbids it (a Task Scheduler task's) it is not started at all, never inside. `spawn_detached`'s own behaviour is
    /// unchanged: it leaves the first job and falls back to starting inside the second.
    #[test]
    fn the_holder_leaves_a_job_that_lets_it_and_never_starts_inside_one_that_does_not() {
        let (strict, old) = job_report("breakaway");
        assert_eq!(strict, "STRICT ok injob=false");
        assert_eq!(old, "OLD in_job=false injob=false");
        let (strict, old) = job_report("locked");
        assert_eq!(strict, "STRICT err PermissionDenied");
        assert_eq!(old, "OLD in_job=true injob=true");
    }

    /// Not a test: the node's side of `a_terminal_is_handed_to_another_process_and_back_without_losing_output`. Reads two handle
    /// values from its input, types into the terminal through the first and reads it through the second.
    #[test]
    fn reader_helper() {
        if std::env::var(ROLE).as_deref() != Ok("reader") {
            return;
        }
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
        let mut ends = line
            .split_whitespace()
            .map(|v| v.parse::<usize>().unwrap() as HANDLE);
        let (input, output) = (ends.next().unwrap(), ends.next().unwrap());
        let typed = b"echo from-reader-%OS%\r\n";
        let mut n = 0u32;
        // SAFETY: handles duplicated into this process for it; valid buffers.
        unsafe {
            WriteFile(
                input,
                typed.as_ptr(),
                typed.len() as u32,
                &mut n,
                null_mut(),
            )
        };
        let mut seen = String::new();
        let mut buf = [0u8; 4096];
        while !seen.contains("from-reader-Windows_NT") {
            let mut n = 0u32;
            // SAFETY: as above.
            if unsafe {
                ReadFile(
                    output,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut n,
                    null_mut(),
                )
            } == 0
                || n == 0
            {
                break;
            }
            seen.push_str(&String::from_utf8_lossy(&buf[..n as usize]));
        }
        println!("READER {}", seen.contains("from-reader-Windows_NT"));
    }

    /// Reads the terminal with a holder's reader, ended and started again, until `needle` was read; returns everything read.
    fn held_until(held: &HeldTerminal, needle: &str) -> String {
        let mut seen = String::new();
        let end = Instant::now() + Duration::from_secs(20);
        while Instant::now() < end && !seen.contains(needle) {
            let reader = held.start_reader().unwrap();
            std::thread::sleep(Duration::from_millis(100));
            seen.push_str(&String::from_utf8_lossy(&reader.end().buffer.into_vec()));
        }
        seen
    }

    /// ADR-0079 decisions 3 and 4: the holder reads a terminal while no node is attached, stops reading, hands the two pipe ends to
    /// another process (opened once, checked by start time), which types into the terminal and reads the answer; then the holder
    /// reads again and sees what comes next. `%OS%` is expanded by `cmd`, so the answer is told from the echo of the typed line.
    #[test]
    fn a_terminal_is_handed_to_another_process_and_back_without_losing_output() {
        let mut held = HeldTerminal::start(&PtyConfig::new(vec!["cmd.exe".into()])).unwrap();
        held.write_input(b"echo before-%OS%\r\n").unwrap();
        let before = held_until(&held, "before-Windows_NT");
        assert!(before.contains("before-Windows_NT"), "{before:?}");
        // Attach: no reader runs now; the other process gets the ends.
        let ends = held.handle().unwrap();
        let mut node = helper("winhold::tests::reader_helper", "reader")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let started = crate::process_start_time(node.id()).unwrap();
        let other = OtherProcess::open(node.id(), started).unwrap();
        let input = other.duplicate_into(ends.input_raw()).unwrap();
        let output = other.duplicate_into(ends.output_raw()).unwrap();
        writeln!(node.stdin.take().unwrap(), "{input} {output}").unwrap();
        let mut report = String::new();
        let mut out = BufReader::new(node.stdout.take().unwrap());
        while !report.contains("READER ") {
            report.clear();
            assert!(
                out.read_line(&mut report).unwrap() > 0,
                "no report from the reader"
            );
        }
        assert!(report.contains("READER true"), "{report}");
        assert!(
            other.ended(Duration::from_secs(20)),
            "the reader did not end"
        );
        let _ = node.wait();
        // Detach: the holder reads again, and nothing written from now on is lost.
        held.write_input(b"echo after-%OS%\r\n").unwrap();
        let after = held_until(&held, "after-Windows_NT");
        assert!(after.contains("after-Windows_NT"), "{after:?}");
        held.kill_and_reap();
        assert!(held.try_wait().unwrap().is_some());
    }

    /// The node's process is checked by its start time on the handle that is then used: a wrong start time (its pid taken by another
    /// process) is refused.
    #[test]
    fn a_process_with_another_start_time_is_refused_and_its_end_is_seen() {
        let mut child = Command::new("ping")
            .args(["-n", "2", "127.0.0.1"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let started = crate::process_start_time(child.id()).unwrap();
        let err = OtherProcess::open(child.id(), started + 1).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        let p = OtherProcess::open(child.id(), started).unwrap();
        assert_eq!(p.start_time().unwrap(), started);
        assert!(!p.ended(Duration::ZERO));
        assert!(p.ended(Duration::from_secs(20)));
        let _ = child.wait();
    }

    /// ADR-0079 decision 5: the holder's pipe tells it which process connected.
    #[test]
    fn the_pipe_server_knows_its_clients_pid() {
        let path = std::env::temp_dir().join(format!("agora-w1-pipe-{}", std::process::id()));
        let listener = crate::windows::listen(&path).unwrap();
        let client = std::thread::spawn(move || crate::windows::connect(&path).unwrap());
        let server = listener.accept().unwrap();
        let _client = client.join().unwrap();
        assert_eq!(server.client_pid().unwrap(), std::process::id());
    }
}

/// M2.6 (ADR-0093 step 3): the holder's program on pipes on Windows.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod held_process_tests {
    use super::HeldProcess;
    use crate::held::HeldSignal;
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};

    /// The environment an agent gets here: nothing of this process's but the system folder (on `PATH` and as `SystemRoot`).
    fn env(extra: &[(&str, &str)]) -> Vec<(String, String)> {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let mut e = vec![
            ("SystemRoot".to_string(), root.clone()),
            ("PATH".to_string(), format!(r"{root}\System32")),
        ];
        e.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        e
    }

    fn argv(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn ended(p: &mut HeldProcess) -> Option<crate::ExitStatus> {
        let end = Instant::now() + Duration::from_secs(20);
        while Instant::now() < end {
            if let Some(s) = p.try_wait().unwrap() {
                return Some(s);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }

    #[test]
    fn lines_go_in_and_out_and_the_end_is_kept() {
        let mut p = HeldProcess::start(&argv(&["findstr", "/R", "^"]), None, &env(&[])).unwrap();
        let (mut input, mut output) = p.take_io().unwrap();
        assert!(p.take_io().is_none(), "the pipes go once");
        input.write_all(b"hello pipes\r\n").unwrap();
        drop(input);
        let mut out = String::new();
        output.read_to_string(&mut out).unwrap();
        assert!(out.contains("hello pipes"), "{out:?}");
        assert_eq!(ended(&mut p).unwrap().code, Some(0));
        // Kept, and a signal is nothing now.
        assert_eq!(p.try_wait().unwrap().unwrap().code, Some(0));
        p.signal(HeldSignal::Kill).unwrap();
    }

    #[test]
    fn it_gets_exactly_the_environment_given() {
        std::env::set_var("AGORA_HELD_PROCESS_LEAK", "from-the-holder");
        let mut p = HeldProcess::start(
            &argv(&[
                "cmd.exe",
                "/d",
                "/c",
                "echo [%FOO%][%AGORA_HELD_PROCESS_LEAK%]",
            ]),
            None,
            &env(&[("FOO", "bar")]),
        )
        .unwrap();
        let (input, mut output) = p.take_io().unwrap();
        drop(input);
        let mut out = String::new();
        output.read_to_string(&mut out).unwrap();
        assert!(out.contains("[bar][%AGORA_HELD_PROCESS_LEAK%]"), "{out:?}");
        assert!(ended(&mut p).is_some());
    }

    #[test]
    fn a_cmd_shim_on_the_given_path_is_found_and_run() {
        let dir = std::env::temp_dir().join(format!("agora-held-shim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("agora-shim.cmd"), "@echo shim-%1\r\n").unwrap();
        let mut e = env(&[]);
        for (k, v) in &mut e {
            if k == "PATH" {
                *v = format!("{};{v}", dir.display());
            }
        }
        let mut p = HeldProcess::start(&argv(&["agora-shim", "arg"]), Some(&dir), &e).unwrap();
        let (input, mut output) = p.take_io().unwrap();
        drop(input);
        let mut out = String::new();
        output.read_to_string(&mut out).unwrap();
        assert!(out.contains("shim-arg"), "{out:?}");
        assert!(ended(&mut p).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kill_ends_the_whole_tree_and_interrupt_does_nothing() {
        let mut p = HeldProcess::start(
            &argv(&["cmd.exe", "/d", "/c", "ping -n 60 127.0.0.1 >nul"]),
            None,
            &env(&[]),
        )
        .unwrap();
        let end = Instant::now() + Duration::from_secs(10);
        let mut kids = Vec::new();
        while kids.is_empty() && Instant::now() < end {
            kids = crate::process_children(p.pid()).unwrap_or_default();
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!kids.is_empty(), "ping did not start");
        p.signal(HeldSignal::Interrupt).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(p.try_wait().unwrap().is_none(), "interrupt ended it");
        p.signal(HeldSignal::Kill).unwrap();
        assert!(ended(&mut p).is_some());
        for k in kids {
            assert!(
                crate::process_start_time(k).is_none(),
                "{k} outlived the kill"
            );
        }
    }

    #[test]
    fn an_agent_that_ends_leaves_nothing_running_in_its_job() {
        // `start /b` leaves ping running after cmd itself ends.
        let mut p = HeldProcess::start(
            &argv(&["cmd.exe", "/d", "/c", "start /b ping -n 60 127.0.0.1 >nul"]),
            None,
            &env(&[]),
        )
        .unwrap();
        let end = Instant::now() + Duration::from_secs(10);
        let mut kids = Vec::new();
        while kids.is_empty() && Instant::now() < end {
            kids = crate::process_children(p.pid()).unwrap_or_default();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!kids.is_empty(), "ping was never seen");
        assert!(ended(&mut p).is_some());
        std::thread::sleep(Duration::from_millis(300));
        for k in kids {
            assert!(
                crate::process_start_time(k).is_none(),
                "{k} outlived its agent"
            );
        }
    }
}
