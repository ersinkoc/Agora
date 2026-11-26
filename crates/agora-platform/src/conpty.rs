//! Windows terminals (ADR-0045): ConPTY. The child runs attached to a pseudo-console fed by two anonymous pipes, inside a job object
//! that kills the whole process tree when it is closed (the Windows counterpart of the Unix process group).
//!
//! There is no foreground process group on Windows: [`Pty::foreground_pid`] is the shell's newest direct child, followed down through
//! shells (ADR-0094 Amendment A, `winproc::foreground`), or the child itself when nothing runs in it.

use std::ffi::{c_void, OsString};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::{Arc, Mutex};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
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
    CreateProcessAsUserW, CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, INFINITE,
    PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};
use windows_sys::Win32::System::IO::CancelIoEx;

use crate::winconfine::Confinement;
use crate::{ExitStatus, Pty, PtyConfig, PtyControl, PtyControlOps};

const CHUNK: usize = 16 * 1024;
const QUEUE: usize = 64;
/// `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`.
const ATTR_PSEUDOCONSOLE: usize = 0x0002_0016;

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
pub(crate) fn quote_arg(arg: &str) -> String {
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

/// Whether `program` names a batch file, which `CreateProcess` runs through `cmd.exe` (a name's trailing dots and spaces are ignored).
fn is_batch(program: &str) -> bool {
    let p = program.trim_end_matches(['.', ' ']).to_ascii_lowercase();
    p.ends_with(".bat") || p.ends_with(".cmd")
}

/// The full path of the batch file `program` names, found before `cmd.exe` is started: given a bare name, `cmd.exe` would look in its
/// own current folder first, which is the agent's project folder, so a `.cmd` planted there would run instead. As the standard
/// library resolves a program: a path with a folder in it is taken relative to this process's folder (as `CreateProcess` took it), a
/// bare name is looked up in the child's `PATH` when `env` sets one, then this program's folder, the system folders and this process's
/// `PATH`, never in a current folder. `InvalidInput` for a NUL in the name, `NotFound` when no such file exists.
fn resolve_batch(program: &str, env: &[(String, String)]) -> io::Result<std::path::PathBuf> {
    use std::path::Path;
    if program.contains('\0') {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    if program.contains(['\\', '/', ':']) {
        let p = std::path::absolute(Path::new(program))?;
        return if p.is_file() {
            Ok(p)
        } else {
            Err(io::ErrorKind::NotFound.into())
        };
    }
    crate::windows::search_dirs(child_path(env))
        .into_iter()
        .map(|d| d.join(program))
        .find(|p| p.is_file())
        .ok_or_else(|| io::ErrorKind::NotFound.into())
}

/// The child's own `PATH`, when its environment sets one.
fn child_path(env: &[(String, String)]) -> Option<&std::ffi::OsStr> {
    env.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
        .map(|(_, path)| std::ffi::OsStr::new(path.as_str()))
}

/// The command line for a batch file. `cmd.exe` reads it, not the C runtime, so [`quote_arg`]'s `\"` would end the quoting and a `&`
/// or `|` after it would run another command (CVE-2024-24576). Built as the standard library builds it since Rust 1.77.2: the script
/// (a full path, from [`resolve_batch`]) run by `cmd.exe /e:ON /v:OFF /d /c "..."`, any argument with an ASCII symbol or a control
/// character quoted, `"` doubled, `%` broken up so no variable expands, in the script's path too. `InvalidInput` for a line break or
/// NUL in an argument ([`crate::batch_argument_refused`]), or a script path `cmd.exe` cannot carry.
fn batch_command_line(script: &std::path::Path, args: &[String]) -> io::Result<String> {
    let invalid = || io::Error::from(io::ErrorKind::InvalidInput);
    let script = script.to_str().ok_or_else(invalid)?;
    if script.contains(['"', '\0']) || script.ends_with('\\') {
        return Err(invalid());
    }
    // `%cd:~,%` is an empty piece of the built-in `cd`: it splits `%VAR%` so `cmd.exe` expands nothing (see the arguments below).
    let script = script.replace('%', "%%cd:~,%");
    let cmd = std::path::PathBuf::from(
        std::env::var_os("SystemRoot").unwrap_or_else(|| OsString::from(r"C:\Windows")),
    )
    .join(r"System32\cmd.exe");
    let mut line = format!(
        "\"{}\" /e:ON /v:OFF /d /c \"\"{script}\"",
        cmd.to_string_lossy()
    );
    for arg in args {
        if arg.contains(['\r', '\n', '\0']) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                crate::BatchArgumentRefused,
            ));
        }
        line.push(' ');
        let quote = arg.is_empty()
            || arg.ends_with('\\')
            || arg.chars().any(|c| {
                c.is_control()
                    || (c.is_ascii() && !(c.is_ascii_alphanumeric() || r"#$*+-./:?@\_".contains(c)))
            });
        if quote {
            line.push('"');
        }
        let mut backslashes = 0;
        for c in arg.chars() {
            if c == '\\' {
                backslashes += 1;
            } else {
                if c == '"' {
                    line.push_str(&"\\".repeat(backslashes));
                    line.push('"');
                } else if c == '%' {
                    // `%cd:~,%` is an empty piece of the built-in `cd`: it splits `%VAR%` so `cmd.exe` expands nothing.
                    line.push_str("%%cd:~,");
                }
                backslashes = 0;
            }
            line.push(c);
        }
        if quote {
            line.push_str(&"\\".repeat(backslashes));
            line.push('"');
        }
    }
    line.push('"');
    Ok(line)
}

/// `vars_os`, not `vars`: Windows allows variables that are not valid Unicode, and `vars` panics on them.
fn environment(extra: &[(String, String)], remove: &[String]) -> Vec<u16> {
    let mut vars: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(k, _)| !crate::env_removed(k, remove))
        .collect();
    let mut set = |k: &str, v: &str| {
        vars.retain(|(have, _)| !have.eq_ignore_ascii_case(k));
        vars.push((k.into(), v.into()));
    };
    if !extra.iter().any(|(k, _)| k.eq_ignore_ascii_case("TERM")) {
        set("TERM", "xterm-256color");
    }
    for (k, v) in extra {
        set(k, v);
    }
    let mut block: Vec<u16> = Vec::new();
    for (k, v) in vars {
        block.extend(k.encode_wide());
        block.push(u16::from(b'='));
        block.extend(v.encode_wide());
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
    /// Last, so the process and job are closed before the folder permissions are taken back.
    _confinement: Option<Confinement>,
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

    /// Only for a process that is over: its code can be 259, the value `STILL_ACTIVE` would mean for a live one.
    fn exit_code(&self) -> Option<i32> {
        let mut code = 0u32;
        // SAFETY: a live process handle.
        let ok = unsafe { GetExitCodeProcess(self.process.0, &mut code) };
        (ok != 0).then_some(code as i32)
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
        crate::winproc::foreground(self.pid)
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
    let (inner, out_read) = start_terminal(config)?;
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
    watch_exit(&inner)?;
    Ok(Box::new(ConPty { inner, output }))
}

/// The pseudo-console, its process and job, and the read end of its output: everything of a terminal but the reading. Shared by
/// [`spawn`] and [`HeldTerminal::start`].
fn start_terminal(config: &PtyConfig) -> io::Result<(Arc<Inner>, Handle)> {
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
    let (process, pid, confinement) = match result {
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
        _confinement: confinement,
    });
    Ok((inner, out_read))
}

/// When the process is over, close the console: that is what ends the output pipe, so the reader sees the end.
fn watch_exit(inner: &Arc<Inner>) -> io::Result<()> {
    let waiter = Arc::clone(inner);
    std::thread::Builder::new()
        .name("agora-pty-waiter".into())
        .spawn(move || {
            // SAFETY: a live process handle (the Arc keeps it open).
            unsafe { WaitForSingleObject(waiter.process.0, INFINITE) };
            waiter.close_console();
        })?;
    Ok(())
}

/// ADR-0079 (decisions 1, 3, 4): a terminal the terminal holder owns. It starts like [`spawn`]'s, but its output is read by the holder
/// only while no node is attached, into a buffer of at most `limit` bytes that drops the oldest first, so the agent never blocks on a
/// full pipe. When a node attaches, the holder stops reading ([`Self::stop_reading`]: a read in flight is cancelled, nothing written
/// to the pipe is lost) and hands it duplicates of the two pipe ends; the pseudo-console itself, its size and its end stay here (an
/// `HPCON` is not a handle another process can use).
#[allow(dead_code)] // ADR-0079 W2 calls it; until then only the tests do.
pub(crate) struct HeldTerminal {
    inner: Arc<Inner>,
    output: Arc<Handle>,
    reader: Option<std::thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    buffer: Arc<Mutex<std::collections::VecDeque<u8>>>,
    limit: usize,
}

#[allow(dead_code)] // ADR-0079 W2 calls it; until then only the tests do.
impl HeldTerminal {
    /// Starts the program of `config` on a new pseudo-console, and starts reading it.
    pub(crate) fn start(config: &PtyConfig, limit: usize) -> io::Result<Self> {
        let (inner, output) = start_terminal(config)?;
        watch_exit(&inner)?;
        let mut held = Self {
            inner,
            output: Arc::new(output),
            reader: None,
            stop: Arc::new(AtomicBool::new(false)),
            buffer: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            limit,
        };
        held.start_reading()?;
        Ok(held)
    }

    /// Reads the output into the buffer again (no node attached). Nothing when it is already reading.
    pub(crate) fn start_reading(&mut self) -> io::Result<()> {
        if self.reader.is_some() {
            return Ok(());
        }
        self.stop.store(false, Ordering::Release);
        let (output, stop, buffer, limit) = (
            Arc::clone(&self.output),
            Arc::clone(&self.stop),
            Arc::clone(&self.buffer),
            self.limit,
        );
        self.reader = Some(
            std::thread::Builder::new()
                .name("agora-held-reader".into())
                .spawn(move || {
                    let mut buf = vec![0u8; CHUNK];
                    while !stop.load(Ordering::Acquire) {
                        let mut n = 0u32;
                        // SAFETY: `buf` is valid for CHUNK bytes; a synchronous pipe handle the Arc keeps open.
                        let ok = unsafe {
                            ReadFile(
                                output.0,
                                buf.as_mut_ptr().cast(),
                                CHUNK as u32,
                                &mut n,
                                null_mut(),
                            )
                        };
                        if ok == 0 || n == 0 {
                            // Cancelled by `stop_reading`, or the end of the output.
                            break;
                        }
                        // Kept even when the read completed as it was being cancelled: those bytes are out of the pipe now.
                        let Ok(mut held) = buffer.lock() else { break };
                        held.extend(&buf[..n as usize]);
                        let over = held.len().saturating_sub(limit);
                        held.drain(..over);
                    }
                })?,
        );
        Ok(())
    }

    /// Stops reading (a node attaches) and returns what was read since the last call. A read in flight is cancelled, so whatever the
    /// agent writes from now on stays in the pipe for the node.
    pub(crate) fn stop_reading(&mut self) -> Vec<u8> {
        if let Some(reader) = self.reader.take() {
            self.stop.store(true, Ordering::Release);
            // The reader may be between two reads when the first cancel comes: cancel until it has ended.
            while !reader.is_finished() {
                // SAFETY: a live pipe handle; cancels the reads in flight on it, from any thread.
                unsafe { CancelIoEx(self.output.0, null()) };
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let _ = reader.join();
        }
        self.buffer
            .lock()
            .map(|mut b| b.drain(..).collect())
            .unwrap_or_default()
    }

    /// The write end of the terminal's input, to duplicate into the node. Valid while `self` lives.
    pub(crate) fn input_handle(&self) -> io::Result<HANDLE> {
        self.inner
            .input
            .lock()
            .map(|h| h.0)
            .map_err(|_| io::Error::other("poisoned"))
    }

    /// The read end of the terminal's output, to duplicate into the node. Valid while `self` lives; read it only when this terminal
    /// is not reading ([`Self::stop_reading`]).
    pub(crate) fn output_handle(&self) -> HANDLE {
        self.output.0
    }

    pub(crate) fn write_input(&self, bytes: &[u8]) -> io::Result<()> {
        self.inner.write_input(bytes)
    }

    pub(crate) fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        self.inner.resize(cols, rows)
    }

    pub(crate) fn kill(&self) -> io::Result<()> {
        self.inner.kill()
    }

    pub(crate) fn pid(&self) -> u32 {
        self.inner.pid
    }

    /// `Some` with the exit code once the agent's process is over.
    pub(crate) fn exited(&self) -> Option<i32> {
        if self.inner.exited() {
            self.inner.exit_code()
        } else {
            None
        }
    }
}

impl Drop for HeldTerminal {
    /// ADR-0079 decision 8: a terminal the holder lets go of ends with it, as a dropped [`spawn`] terminal does.
    fn drop(&mut self) {
        let _ = self.inner.kill();
        let _ = self.stop_reading();
    }
}

fn start_process(config: &PtyConfig, hpc: HPCON) -> io::Result<(Handle, u32, Option<Confinement>)> {
    // A bare name installed as a `.cmd` shim (`npx`) becomes its full path; `CreateProcess` alone adds only `.exe`.
    let first = config.argv.first().map(|p| {
        crate::windows::program_path(p, child_path(&config.env))
            .to_str()
            .map_or_else(|| p.clone(), str::to_string)
    });
    let mut cmdline = wide(&match first.as_deref() {
        Some(program) if is_batch(program) => {
            batch_command_line(&resolve_batch(program, &config.env)?, &config.argv[1..])?
        }
        Some(program) => std::iter::once(program)
            .chain(config.argv[1..].iter().map(String::as_str))
            .map(quote_arg)
            .collect::<Vec<_>>()
            .join(" "),
        None => String::new(),
    });
    let confinement = config
        .sandbox
        .as_ref()
        .map(|s| Confinement::new(s, config.identity.as_deref()))
        .transpose()?;
    let env = match confinement.as_ref().and_then(Confinement::git_config) {
        Some(file) => {
            let mut extra = config.env.clone();
            extra.extend(crate::git_config_env(file));
            environment(&extra, &config.env_remove)
        }
        None => environment(&config.env, &config.env_remove),
    };
    // Not lossily: a folder name need not be valid Unicode.
    let cwd: Option<Vec<u16>> = config.cwd.as_ref().map(|c| {
        c.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    });

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
    // This process's own standard streams may be redirected (a test harness, a service). Saying "no handles" makes the child bind to
    // the pseudo-console instead of inheriting them.
    si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    si.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
    si.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
    si.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
    // SAFETY: an all-zero PROCESS_INFORMATION is a valid out-buffer.
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: every pointer is valid for the call; `cmdline` is mutable as the API requires.
    let flags = EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT;
    let created = unsafe {
        match &confinement {
            Some(c) => CreateProcessAsUserW(
                c.token(),
                null(),
                cmdline.as_mut_ptr(),
                null(),
                null(),
                0,
                flags,
                env.as_ptr().cast(),
                cwd.as_ref().map_or(null(), |c| c.as_ptr()),
                &si.StartupInfo,
                &mut pi,
            ),
            None => CreateProcessW(
                null(),
                cmdline.as_mut_ptr(),
                null(),
                null(),
                0,
                flags,
                env.as_ptr().cast(),
                cwd.as_ref().map_or(null(), |c| c.as_ptr()),
                &si.StartupInfo,
                &mut pi,
            ),
        }
    };
    let error = (created == 0).then(last_error);
    // SAFETY: initialised above; the child has its copy by now.
    unsafe { DeleteProcThreadAttributeList(list) };
    if let Some(e) = error {
        return Err(e);
    }
    drop(Handle(pi.hThread));
    Ok((Handle(pi.hProcess), pi.dwProcessId, confinement))
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
        crate::winproc::foreground(self.inner.pid)
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
    fn an_argument_to_a_batch_file_runs_no_second_command() {
        // CVE-2024-24576: `cmd.exe` reads a batch file's command line, where the C runtime's `\"` ends the quoting.
        assert!(is_batch(r"C:\tools\Run.CMD. "));
        assert!(!is_batch("cmd.exe"));
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("agora-conpty-bat-{}-{nanos}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("run.cmd");
        std::fs::write(&script, "@exit /b 5\r\n").unwrap();
        let mut cfg = PtyConfig::new(vec![
            script.display().to_string(),
            "a\"&type nul>injected.txt&\"b".into(),
            r"C:\R&D\dir\".into(),
        ]);
        cfg.cwd = Some(dir.clone());
        let mut p = spawn(&cfg).unwrap();
        assert_eq!(p.wait().unwrap().code, Some(5));
        assert!(!dir.join("injected.txt").exists());
        // A `%VAR%` in an argument is not expanded either, even where the variable holds `"&type nul>x&"`: the script receives the
        // literal text (exit 8) and no second command ran.
        let literal = dir.join("literal.cmd");
        std::fs::write(
            &literal,
            "@if \"%~1\"==\"%%AGORA_CONPTY_T%%\" (exit /b 8)\r\n@exit /b 9\r\n",
        )
        .unwrap();
        let mut cfg = PtyConfig::new(vec![
            literal.display().to_string(),
            "%AGORA_CONPTY_T%".into(),
        ]);
        cfg.env = vec![("AGORA_CONPTY_T".into(), "\"&type nul>x&\"".into())];
        cfg.cwd = Some(dir.clone());
        assert_eq!(spawn(&cfg).unwrap().wait().unwrap().code, Some(8));
        assert!(!dir.join("x").exists());
        // A line break cannot be carried to a batch file: refused, nothing started.
        let mut cfg = PtyConfig::new(vec![script.display().to_string(), "a\r\nb".into()]);
        cfg.cwd = Some(dir.clone());
        let refused = spawn(&cfg).err().unwrap();
        assert_eq!(refused.kind(), io::ErrorKind::InvalidInput);
        assert!(crate::batch_argument_refused(&refused));
        // Nor a NUL in the script's name.
        let cfg = PtyConfig::new(vec!["a\0b.cmd".into()]);
        assert_eq!(
            spawn(&cfg).err().map(|e| e.kind()),
            Some(io::ErrorKind::InvalidInput)
        );
        // A `%VAR%` in the script's folder is not expanded: the script at that literal path runs.
        let pct = dir.join("p%AGORA_CONPTY_T%q");
        std::fs::create_dir_all(&pct).unwrap();
        std::fs::write(pct.join("run.cmd"), "@exit /b 6\r\n").unwrap();
        let mut cfg = PtyConfig::new(vec![pct.join("run.cmd").display().to_string()]);
        cfg.env = vec![("AGORA_CONPTY_T".into(), "x".into())];
        cfg.cwd = Some(dir.clone());
        assert_eq!(spawn(&cfg).unwrap().wait().unwrap().code, Some(6));
        // A bare name is looked up on PATH, never in the agent's folder, where a planted script of that name sits.
        let tools = dir.join("tools");
        let project = dir.join("project");
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(tools.join("agora-conpty-tool.cmd"), "@exit /b 7\r\n").unwrap();
        std::fs::write(project.join("agora-conpty-tool.cmd"), "@exit /b 9\r\n").unwrap();
        let mut cfg = PtyConfig::new(vec!["agora-conpty-tool.cmd".into()]);
        cfg.env = vec![("PATH".into(), tools.display().to_string())];
        // `cmd.exe` skips its current folder only when this is set, as it may be in the environment the tests run in.
        cfg.env_remove = vec!["NoDefaultCurrentDirectoryInExePath".into()];
        cfg.cwd = Some(project.clone());
        assert_eq!(spawn(&cfg).unwrap().wait().unwrap().code, Some(7));
        // Found nowhere on PATH: not started from the agent's folder either.
        let mut cfg = PtyConfig::new(vec!["agora-conpty-tool.cmd".into()]);
        cfg.env = vec![("PATH".into(), dir.join("empty").display().to_string())];
        cfg.cwd = Some(project);
        assert_eq!(
            spawn(&cfg).err().map(|e| e.kind()),
            Some(io::ErrorKind::NotFound)
        );
        // A bare name with no extension, installed as a `.cmd` shim (`npx`): `CreateProcess` alone would add only `.exe`. It runs the
        // shim on PATH, not one planted in the agent's folder, and its argument is quoted for `cmd.exe`.
        let project = dir.join("project");
        std::fs::write(tools.join("agora-conpty-shim.cmd"), "@exit /b 4\r\n").unwrap();
        std::fs::write(project.join("agora-conpty-shim.cmd"), "@exit /b 9\r\n").unwrap();
        let mut cfg = PtyConfig::new(vec![
            "agora-conpty-shim".into(),
            "a\"&type nul>shim-injected.txt&\"b".into(),
        ]);
        cfg.env = vec![("PATH".into(), tools.display().to_string())];
        cfg.env_remove = vec!["NoDefaultCurrentDirectoryInExePath".into()];
        cfg.cwd = Some(project.clone());
        assert_eq!(spawn(&cfg).unwrap().wait().unwrap().code, Some(4));
        assert!(!project.join("shim-injected.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
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

    /// ADR-0094 Amendment A on a real pseudoconsole: a program typed into the shell is in front (for the `Pty` and its control alike),
    /// and the shell is again once it ends.
    #[test]
    fn a_program_typed_into_the_shell_is_in_front_until_it_ends() {
        fn front_is(p: &dyn Pty, want: &dyn Fn(u32, &str) -> bool) -> bool {
            let end = Instant::now() + Duration::from_secs(20);
            while Instant::now() < end {
                // Drained, so the console never waits on a full pipe.
                while p.output().try_recv().is_ok() {}
                let a = p.foreground_pid().unwrap();
                let b = p.control().unwrap().foreground_pid().unwrap();
                let name = crate::winproc::name(a).unwrap_or_default();
                if a == b && want(a, &name) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            false
        }
        let mut p = cmd(&[]);
        let shell = p.pid();
        assert!(front_is(&*p, &|pid, _| pid == shell), "the shell first");
        // About 4 s, long enough to be seen and short enough to end on its own.
        p.write_input(b"ping -n 5 127.0.0.1\r").unwrap();
        assert!(
            front_is(&*p, &|pid, name| pid != shell
                && name.eq_ignore_ascii_case("ping")),
            "ping in front"
        );
        assert!(front_is(&*p, &|pid, _| pid == shell), "the shell again");
        p.kill().unwrap();
        p.wait().unwrap();
    }

    #[test]
    fn an_exit_code_of_259_is_an_exit_not_still_running() {
        // 259 is what `GetExitCodeProcess` also answers for a live process (`STILL_ACTIVE`).
        let mut p = cmd(&["/c", "exit 259"]);
        assert_eq!(p.wait().unwrap().code, Some(259));
        assert_eq!(p.try_wait().unwrap().and_then(|s| s.code), Some(259));
    }

    #[test]
    fn a_working_folder_whose_name_is_not_valid_unicode_is_used_as_is() {
        use std::os::windows::ffi::OsStringExt;
        let mut name: Vec<u16> = format!("agora-conpty-{}-", std::process::id())
            .encode_utf16()
            .collect();
        name.push(0xD800);
        let dir = std::env::temp_dir().join(OsString::from_wide(&name));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("marker.txt"), "x").unwrap();
        let mut cfg = PtyConfig::new(vec![
            "cmd.exe".into(),
            "/c".into(),
            "if exist marker.txt (exit 3) else (exit 4)".into(),
        ]);
        cfg.cwd = Some(dir.clone());
        let mut p = spawn(&cfg).unwrap();
        assert_eq!(p.wait().unwrap().code, Some(3));
        drop(p);
        let _ = std::fs::remove_dir_all(&dir);
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
