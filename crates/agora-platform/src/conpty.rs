//! Windows terminals (ADR-0045): ConPTY. The child runs attached to a pseudo-console fed by two anonymous pipes, inside a job object
//! that kills the whole process tree when it is closed (the Windows counterpart of the Unix process group).
//!
//! There is no foreground process group on Windows, so [`Pty::foreground_pid`] is the child itself.

use std::ffi::c_void;
use std::io;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::{Arc, Mutex};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows_sys::Win32::System::Console::{
    ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole, COORD, HPCON,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    JOB_OBJECT_LIMIT_PROCESS_TIME,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, INFINITE,
    PROCESS_INFORMATION, STARTUPINFOEXW,
};

use crate::{ExitStatus, Pty, PtyConfig, PtyControl, PtyControlOps};

const CHUNK: usize = 16 * 1024;
const QUEUE: usize = 64;
/// `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`.
const ATTR_PSEUDOCONSOLE: usize = 0x0002_0016;
const STILL_ACTIVE: u32 = 259;

fn last_error() -> io::Error {
    // SAFETY: plain call.
    io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct Handle(HANDLE);

// SAFETY: a handle is an opaque id the system lets any thread use.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: owned and closed once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

fn pipe() -> io::Result<(Handle, Handle)> {
    let (mut r, mut w): (HANDLE, HANDLE) = (null_mut(), null_mut());
    // SAFETY: valid out pointers; default attributes (not inheritable) and buffer size.
    if unsafe { CreatePipe(&mut r, &mut w, null(), 0) } == 0 {
        return Err(last_error());
    }
    Ok((Handle(r), Handle(w)))
}

/// One argument as the C runtime's command-line parser reads it back (the rules of `CommandLineToArgvW`).
fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\x0b', '"']) {
        return arg.to_string();
    }
    let mut out = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                out.push(c);
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

fn environment(extra: &[(String, String)]) -> Vec<u16> {
    let mut vars: Vec<(String, String)> = std::env::vars().collect();
    let mut set = |k: &str, v: &str| {
        vars.retain(|(have, _)| !have.eq_ignore_ascii_case(k));
        vars.push((k.to_string(), v.to_string()));
    };
    if !extra.iter().any(|(k, _)| k.eq_ignore_ascii_case("TERM")) {
        set("TERM", "xterm-256color");
    }
    for (k, v) in extra {
        set(k, v);
    }
    let mut block: Vec<u16> = Vec::new();
    for (k, v) in vars {
        block.extend(format!("{k}={v}").encode_utf16());
        block.push(0);
    }
    block.push(0);
    block
}

struct Inner {
    input: Mutex<Handle>,
    hpc: HPCON,
    console_closed: AtomicBool,
    process: Handle,
    job: Option<Handle>,
    pid: u32,
}

// SAFETY: the pseudo-console handle is an opaque id; every use of it is a thread-safe system call.
unsafe impl Send for Inner {}
unsafe impl Sync for Inner {}

impl Inner {
    fn close_console(&self) {
        if !self.console_closed.swap(true, Ordering::AcqRel) {
            // SAFETY: closed once; older Windows waits here until the output is drained, which the reader thread does.
            unsafe { ClosePseudoConsole(self.hpc) };
        }
    }

    fn exit_code(&self) -> Option<i32> {
        let mut code = 0u32;
        // SAFETY: a live process handle.
        let ok = unsafe { GetExitCodeProcess(self.process.0, &mut code) };
        (ok != 0 && code != STILL_ACTIVE).then_some(code as i32)
    }

    fn exited(&self) -> bool {
        // SAFETY: a live process handle.
        unsafe { WaitForSingleObject(self.process.0, 0) == WAIT_OBJECT_0 }
    }
}

impl PtyControlOps for Inner {
    fn write_input(&self, bytes: &[u8]) -> io::Result<()> {
        let input = self
            .input
            .lock()
            .map_err(|_| io::Error::other("poisoned"))?;
        let mut rest = bytes;
        while !rest.is_empty() {
            let mut n = 0u32;
            let len = u32::try_from(rest.len()).unwrap_or(u32::MAX);
            // SAFETY: `rest` is valid for `len` bytes; a synchronous pipe handle, so no OVERLAPPED.
            if unsafe { WriteFile(input.0, rest.as_ptr().cast(), len, &mut n, null_mut()) } == 0 {
                return Err(last_error());
            }
            rest = &rest[n as usize..];
        }
        Ok(())
    }

    fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        let size = COORD {
            X: i16::try_from(cols).unwrap_or(i16::MAX),
            Y: i16::try_from(rows).unwrap_or(i16::MAX),
        };
        // SAFETY: a live pseudo-console.
        let hr = unsafe { ResizePseudoConsole(self.hpc, size) };
        if hr == 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(hr))
        }
    }

    fn kill(&self) -> io::Result<()> {
        if self.exited() {
            return Ok(());
        }
        // SAFETY: live handles; ending the job ends every process in it.
        unsafe {
            match &self.job {
                Some(j) => TerminateJobObject(j.0, 1),
                None => TerminateProcess(self.process.0, 1),
            };
        }
        Ok(())
    }

    fn foreground_pid(&self) -> io::Result<u32> {
        Ok(self.pid)
    }

    fn pid(&self) -> u32 {
        self.pid
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if !self.console_closed.load(Ordering::Acquire) {
            // Closed elsewhere so a console that waits for its output to drain cannot block whoever dropped this.
            let hpc = self.hpc as usize;
            let _ = std::thread::Builder::new().spawn(move || {
                // SAFETY: the console was not closed before; this is the only close.
                unsafe { ClosePseudoConsole(hpc as HPCON) };
            });
        }
    }
}

struct ConPty {
    inner: Arc<Inner>,
    output: Receiver<Vec<u8>>,
}

pub(crate) fn spawn(config: &PtyConfig) -> io::Result<Box<dyn Pty>> {
    let (in_read, in_write) = pipe()?;
    let (out_read, out_write) = pipe()?;
    let size = COORD {
        X: i16::try_from(config.cols).unwrap_or(80),
        Y: i16::try_from(config.rows).unwrap_or(24),
    };
    let mut hpc: HPCON = 0;
    // SAFETY: valid pipe ends and out pointer.
    let hr = unsafe { CreatePseudoConsole(size, in_read.0, out_write.0, 0, &mut hpc) };
    if hr != 0 {
        return Err(io::Error::from_raw_os_error(hr));
    }
    // The pseudo-console keeps its own references to these two ends.
    drop(in_read);
    drop(out_write);

    let result = start_process(config, hpc);
    let (process, pid) = match result {
        Ok(p) => p,
        Err(e) => {
            // SAFETY: the console was created above and is closed once.
            unsafe { ClosePseudoConsole(hpc) };
            return Err(e);
        }
    };
    let job = make_job(config, process.0);
    let inner = Arc::new(Inner {
        input: Mutex::new(in_write),
        hpc,
        console_closed: AtomicBool::new(false),
        process,
        job,
        pid,
    });
    let (tx, output) = sync_channel::<Vec<u8>>(QUEUE);
    let read_handle = out_read;
    std::thread::Builder::new()
        .name("agora-pty-reader".into())
        .spawn(move || {
            // Captures the whole handle (which is `Send`), not just its raw pointer field.
            let read_handle = read_handle;
            let mut buf = vec![0u8; CHUNK];
            loop {
                let mut n = 0u32;
                // SAFETY: `buf` is valid for CHUNK bytes; a synchronous pipe handle.
                let ok = unsafe {
                    ReadFile(
                        read_handle.0,
                        buf.as_mut_ptr().cast(),
                        CHUNK as u32,
                        &mut n,
                        null_mut(),
                    )
                };
                if ok == 0 || n == 0 || tx.send(buf[..n as usize].to_vec()).is_err() {
                    break;
                }
            }
        })?;
    // When the process is over, close the console: that is what ends the output pipe, so the reader sees the end.
    let waiter = Arc::clone(&inner);
    std::thread::Builder::new()
        .name("agora-pty-waiter".into())
        .spawn(move || {
            // SAFETY: a live process handle (the Arc keeps it open).
            unsafe { WaitForSingleObject(waiter.process.0, INFINITE) };
            waiter.close_console();
        })?;
    Ok(Box::new(ConPty { inner, output }))
}

fn start_process(config: &PtyConfig, hpc: HPCON) -> io::Result<(Handle, u32)> {
    let mut cmdline = wide(
        &config
            .argv
            .iter()
            .map(|a| quote_arg(a))
            .collect::<Vec<_>>()
            .join(" "),
    );
    let env = environment(&config.env);
    let cwd = config.cwd.as_ref().map(|c| wide(&c.to_string_lossy()));

    let mut size = 0usize;
    // SAFETY: the documented size query: null list, one attribute.
    unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut size) };
    let mut storage = vec![0u8; size];
    let list = storage.as_mut_ptr().cast::<c_void>();
    // SAFETY: `storage` is `size` bytes and outlives the list.
    if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) } == 0 {
        return Err(last_error());
    }
    // SAFETY: the attribute value is the console handle itself, as the API wants.
    let ok = unsafe {
        UpdateProcThreadAttribute(
            list,
            0,
            ATTR_PSEUDOCONSOLE,
            hpc as *const c_void,
            std::mem::size_of::<HPCON>(),
            null_mut(),
            null(),
        )
    };
    if ok == 0 {
        let e = last_error();
        // SAFETY: initialised above.
        unsafe { DeleteProcThreadAttributeList(list) };
        return Err(e);
    }
    // SAFETY: an all-zero STARTUPINFOEXW is the documented starting point.
    let mut si: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    si.lpAttributeList = list;
    // SAFETY: an all-zero PROCESS_INFORMATION is a valid out-buffer.
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: every pointer is valid for the call; `cmdline` is mutable as the API requires.
    let created = unsafe {
        CreateProcessW(
            null(),
            cmdline.as_mut_ptr(),
            null(),
            null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            env.as_ptr().cast(),
            cwd.as_ref().map_or(null(), |c| c.as_ptr()),
            &si.StartupInfo,
            &mut pi,
        )
    };
    let error = (created == 0).then(last_error);
    // SAFETY: initialised above; the child has its copy by now.
    unsafe { DeleteProcThreadAttributeList(list) };
    if let Some(e) = error {
        return Err(e);
    }
    drop(Handle(pi.hThread));
    Ok((Handle(pi.hProcess), pi.dwProcessId))
}

/// A job that ends every process in it when it is closed, with the memory and CPU limits of the request where they are set.
fn make_job(config: &PtyConfig, process: HANDLE) -> Option<Handle> {
    // SAFETY: plain call.
    let job = unsafe { CreateJobObjectW(null(), null()) };
    if job.is_null() {
        return None;
    }
    let job = Handle(job);
    // SAFETY: an all-zero limit structure means "no limits".
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if let Some(bytes) = config.limits.memory_bytes {
        info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        info.ProcessMemoryLimit = usize::try_from(bytes).unwrap_or(usize::MAX);
    }
    if let Some(secs) = config.limits.cpu_seconds {
        info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_TIME;
        // In units of 100 ns.
        info.BasicLimitInformation.PerProcessUserTimeLimit =
            i64::try_from(secs.saturating_mul(10_000_000)).unwrap_or(i64::MAX);
    }
    // SAFETY: `info` is the structure the class names, with its size.
    let set = unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    // SAFETY: live handles.
    let assigned = unsafe { AssignProcessToJobObject(job.0, process) };
    (set != 0 && assigned != 0).then_some(job)
}

fn status(code: i32) -> ExitStatus {
    ExitStatus {
        code: Some(code),
        signal: None,
    }
}

impl Pty for ConPty {
    fn output(&self) -> &Receiver<Vec<u8>> {
        &self.output
    }

    fn write_input(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.inner.write_input(bytes)
    }

    fn resize(&mut self, cols: u16, rows: u16) -> io::Result<()> {
        self.inner.resize(cols, rows)
    }

    fn pid(&self) -> u32 {
        self.inner.pid
    }

    fn kill(&mut self) -> io::Result<()> {
        self.inner.kill()
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Ok(if self.inner.exited() {
            self.inner.exit_code().map(status)
        } else {
            None
        })
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        // SAFETY: a live process handle.
        unsafe { WaitForSingleObject(self.inner.process.0, INFINITE) };
        self.inner.exit_code().map(status).ok_or_else(last_error)
    }

    fn foreground_pid(&self) -> io::Result<u32> {
        Ok(self.inner.pid)
    }

    fn control(&self) -> io::Result<PtyControl> {
        Ok(PtyControl {
            inner: self.inner.clone(),
        })
    }
}

impl Drop for ConPty {
    fn drop(&mut self) {
        let _ = self.inner.kill();
        // The reader then finds nobody listening and ends; the process is not waited for here.
        self.output = sync_channel(1).1;
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn read_until(pty: &dyn Pty, needle: &str) -> String {
        let mut seen = String::new();
        let end = Instant::now() + Duration::from_secs(20);
        while Instant::now() < end && !seen.contains(needle) {
            if let Ok(c) = pty.output().recv_timeout(Duration::from_millis(100)) {
                seen.push_str(&String::from_utf8_lossy(&c));
            }
        }
        seen
    }

    fn cmd(args: &[&str]) -> Box<dyn Pty> {
        let mut argv = vec!["cmd.exe".to_string()];
        argv.extend(args.iter().map(|s| (*s).to_string()));
        spawn(&PtyConfig::new(argv)).expect("spawn")
    }

    #[test]
    fn arguments_are_quoted_so_they_read_back_unchanged() {
        assert_eq!(quote_arg("plain"), "plain");
        assert_eq!(quote_arg("with space"), "\"with space\"");
        assert_eq!(quote_arg(""), "\"\"");
        assert_eq!(quote_arg("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quote_arg("dir\\ name\\"), "\"dir\\ name\\\\\"");
    }

    #[test]
    fn a_program_in_a_console_prints_and_its_exit_code_comes_back() {
        let mut p = cmd(&["/c", "echo agora-conpty-ok & exit 7"]);
        let seen = read_until(&*p, "agora-conpty-ok");
        assert!(
            seen.contains("agora-conpty-ok"),
            "{seen:?} exited={:?}",
            p.try_wait()
        );
        let status = p.wait().unwrap();
        assert_eq!(status.code, Some(7));
        // The output channel ends once the process is over.
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            match p.output().recv_timeout(Duration::from_millis(100)) {
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                _ => assert!(Instant::now() < end, "the output never ended"),
            }
        }
    }

    #[test]
    fn input_reaches_the_program_and_resize_and_kill_work() {
        let mut p = cmd(&[]);
        p.write_input(b"echo typed-%OS%-line\r\n").unwrap();
        let seen = read_until(&*p, "typed-Windows_NT-line");
        assert!(
            seen.contains("typed-Windows_NT-line"),
            "{seen:?} exited={:?}",
            p.try_wait()
        );
        p.resize(100, 30).unwrap();
        assert!(p.try_wait().unwrap().is_none());
        let control = p.control().unwrap();
        control.kill().unwrap();
        assert!(p.wait().unwrap().code.is_some());
        assert!(p.try_wait().unwrap().is_some());
    }

    #[test]
    fn killing_a_terminal_ends_the_programs_it_started_too() {
        let mut p = cmd(&["/c", "ping -n 60 127.0.0.1 >nul"]);
        std::thread::sleep(Duration::from_millis(1500));
        let started = Instant::now();
        p.kill().unwrap();
        p.wait().unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
