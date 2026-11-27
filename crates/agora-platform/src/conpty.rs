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
    CloseHandle, DuplicateHandle, GetLastError, DUPLICATE_SAME_ACCESS, ERROR_BROKEN_PIPE,
    ERROR_OPERATION_ABORTED, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{GetFileType, ReadFile, WriteFile, FILE_TYPE_PIPE};
use windows_sys::Win32::System::Console::{
    ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole, SetConsoleCtrlHandler, COORD,
    HPCON,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    JOB_OBJECT_LIMIT_PROCESS_TIME,
};
use windows_sys::Win32::System::Pipes::{CreatePipe, PeekNamedPipe};
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess,
    GetExitCodeProcess, InitializeProcThreadAttributeList, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, INFINITE, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
    STARTUPINFOEXW,
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

#[derive(Debug)]
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
/// Lets what a terminal starts take Ctrl-C, whatever this process inherited. A process started with `CREATE_NEW_PROCESS_GROUP` has Ctrl-C
/// turned off, and passes that on to everything it starts: every detached start does that ([`crate::winproc::DETACHED_FLAGS`]: the
/// node under its supervisor, the front door's node, the holder), so a `ping` or a build in an agent's terminal never stopped at the
/// Ctrl-C typed into it (measured on Windows 11: `0x03` into the pseudo-console stopped `ping -t` only after this). A detached process
/// has no console of its own, so this changes nothing for it; asked before every start (one cheap call), so nothing that turned it
/// off again in between can keep it from an agent.
fn let_ctrl_c_reach_agents() {
    // SAFETY: plain call; with no handler, FALSE restores normal Ctrl-C processing for this process and what it starts.
    unsafe { SetConsoleCtrlHandler(None, 0) };
}

fn start_terminal(config: &PtyConfig) -> io::Result<(Arc<Inner>, Handle)> {
    let_ctrl_c_reach_agents();
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

// ----- ADR-0079 M2.6: the holder's terminals, in the shape of the Unix holder's (`unixhold.rs`) -----

/// Duplicates `handle` within this process (same access, not inheritable).
fn duplicate_here(handle: HANDLE) -> io::Result<Handle> {
    let mut copy: HANDLE = null_mut();
    // SAFETY: `handle` is live in this process; this process's pseudo-handle is valid; a valid out pointer.
    let ok = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            handle,
            GetCurrentProcess(),
            &mut copy,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Handle(copy))
}

/// A held terminal's two pipe ends as one passable thing: this process's duplicates of the input's write end and the output's read end
/// (ADR-0079 decision 3). The pseudo-console itself never leaves the holder (an `HPCON` is not a handle another process can use).
#[derive(Debug)]
pub struct TerminalHandle {
    input: Handle,
    output: Handle,
}

impl TerminalHandle {
    /// The terminal's output to read as a file; its input end is closed.
    pub fn into_file(self) -> io::Result<std::fs::File> {
        use std::os::windows::io::FromRawHandle;
        let Self { input, output } = self;
        drop(input);
        let raw = output.0;
        std::mem::forget(output);
        // SAFETY: an owned pipe handle, given up by `output` just above, so it is closed once, by the file.
        Ok(unsafe { std::fs::File::from_raw_handle(raw.cast()) })
    }

    /// M2.6 (the holder link's `StartedRef`/`TerminalRef`): duplicates the two pipe ends into `peer`, the process the holder admitted
    /// and checked by its start time (ADR-0079 decision 5), and gives the reference the frame carries. The duplicates belong to `peer`
    /// from now on; until [`ExportedTerminal::delivered`] says the frame went out, dropping the export closes them there again, so a
    /// frame that never left (or a link that dropped first) leaves nothing behind in the node.
    pub fn export_to<'a>(&self, peer: &'a crate::ProcessWatch) -> io::Result<ExportedTerminal<'a>> {
        let process = peer.process();
        let input = process.duplicate_into(self.input.0)?;
        let output = match process.duplicate_into(self.output.0) {
            Ok(v) => v,
            Err(e) => {
                let _ = process.close_theirs(input);
                return Err(e);
            }
        };
        let mut bytes = Vec::with_capacity(TERMINAL_REF_LEN);
        bytes.push(TERMINAL_REF_VERSION);
        bytes.extend_from_slice(&(input as u64).to_be_bytes());
        bytes.extend_from_slice(&(output as u64).to_be_bytes());
        Ok(ExportedTerminal {
            peer,
            values: Some([input, output]),
            bytes,
        })
    }

    /// M2.6: the terminal a holder's reference names, in this process. Checked before anything is owned: the exact length and version,
    /// two values that are handles of this process, and both of them pipes. A reference that fails any of that is refused and nothing
    /// of it is closed (its values may name handles this process holds for something else).
    pub fn import(reference: &[u8]) -> io::Result<TerminalHandle> {
        let invalid = || io::Error::new(io::ErrorKind::InvalidData, "terminal reference");
        if reference.len() != TERMINAL_REF_LEN || reference[0] != TERMINAL_REF_VERSION {
            return Err(invalid());
        }
        let value = |at: usize| -> io::Result<HANDLE> {
            let mut raw = [0u8; 8];
            raw.copy_from_slice(&reference[at..at + 8]);
            let v = usize::try_from(u64::from_be_bytes(raw)).map_err(|_| invalid())?;
            // Zero and INVALID_HANDLE_VALUE are never a duplicated handle.
            if v == 0 || v as HANDLE == INVALID_HANDLE_VALUE {
                return Err(invalid());
            }
            Ok(v as HANDLE)
        };
        let (input, output) = (value(1)?, value(9)?);
        // SAFETY: GetFileType only reads the handle's type; a value that is no handle of this process gives FILE_TYPE_UNKNOWN.
        let pipe = |h: HANDLE| unsafe { GetFileType(h) } == FILE_TYPE_PIPE;
        if input == output || !pipe(input) || !pipe(output) {
            return Err(invalid());
        }
        Ok(TerminalHandle {
            input: Handle(input),
            output: Handle(output),
        })
    }

    #[allow(dead_code)] // M2.6's terminal transfer (`export_to`) duplicates these into the node; until then only the tests do.
    pub(crate) fn input_raw(&self) -> HANDLE {
        self.input.0
    }

    #[allow(dead_code)] // As `input_raw`.
    pub(crate) fn output_raw(&self) -> HANDLE {
        self.output.0
    }
}

/// The version byte of a terminal reference (M2.6), and its whole length: the version, then the input's and the output's handle values
/// in the node, each a `u64` big-endian. Private to Windows; the holder link carries it as opaque bytes of at most
/// [`crate::TERMINAL_REF_MAX`].
const TERMINAL_REF_VERSION: u8 = 1;
const TERMINAL_REF_LEN: usize = 17;

/// A terminal duplicated into the node for one frame ([`TerminalHandle::export_to`]). Until [`Self::delivered`], dropping it closes
/// the duplicates in the node again.
pub struct ExportedTerminal<'a> {
    peer: &'a crate::ProcessWatch,
    values: Option<[usize; 2]>,
    bytes: Vec<u8>,
}

impl ExportedTerminal<'_> {
    /// The reference to put in the frame.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The frame went out: the node owns the duplicates now.
    pub fn delivered(mut self) {
        self.values = None;
    }
}

impl Drop for ExportedTerminal<'_> {
    fn drop(&mut self) {
        if let Some(values) = self.values.take() {
            for v in values {
                let _ = self.peer.process().close_theirs(v);
            }
        }
    }
}

/// ADR-0079 (decisions 1, 3, 4): a program on a pseudo-console the terminal holder owns. Nothing reads its output until the holder starts
/// a [`TerminalReader`] (no node attached) or [`Self::drain`]s it; a node attached gets duplicates of the two pipe ends
/// ([`Self::handle`]) and reads and writes them itself. The pseudo-console, its size and its end stay here.
pub struct HeldTerminal {
    inner: Arc<Inner>,
    output: Arc<Handle>,
    exit: Option<ExitStatus>,
}

impl HeldTerminal {
    /// Starts the program of `config` on a new pseudo-console, in a kill-on-close job with the request's limits.
    pub fn start(config: &PtyConfig) -> io::Result<Self> {
        let (inner, output) = start_terminal(config)?;
        watch_exit(&inner)?;
        Ok(Self {
            inner,
            output: Arc::new(output),
            exit: None,
        })
    }

    pub fn pid(&self) -> u32 {
        self.inner.pid
    }

    /// Ends the agent (its whole job) and waits for its process, so nothing is left of an agent that never got to a node.
    pub fn kill_and_reap(&mut self) {
        let _ = self.inner.kill();
        // SAFETY: a live process handle (`inner` keeps it open).
        unsafe { WaitForSingleObject(self.inner.process.0, 5_000) };
        let _ = self.try_wait();
    }

    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        self.inner.resize(cols, rows)
    }

    /// What a signal is on Windows (M2.6): `Interrupt` types Ctrl-C (`0x03`) into the pseudo-console, which raises `CTRL_C_EVENT` in
    /// its console processes; `Hangup` and `Terminate` close the pseudo-console, which sends them `CTRL_CLOSE_EVENT` and ends what does
    /// not end by itself (the holder's grace before `Kill` is its own); `Kill` ends the agent's whole job. Nothing once the agent ended.
    pub fn signal(&self, signal: crate::held::HeldSignal) -> io::Result<()> {
        use crate::held::HeldSignal;
        if self.exit.is_some() || self.inner.exited() {
            return Ok(());
        }
        match signal {
            HeldSignal::Interrupt => self.inner.write_input(&[0x03]),
            HeldSignal::Hangup | HeldSignal::Terminate => {
                self.inner.close_console();
                Ok(())
            }
            HeldSignal::Kill => self.inner.kill(),
        }
    }

    /// The agent's exit status once it ended, kept from then on.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.exit.is_none() && self.inner.exited() {
            self.exit = Some(status(self.inner.exit_code().unwrap_or(1)));
        }
        Ok(self.exit)
    }

    /// Duplicates of the two pipe ends, for the node; the holder keeps its own.
    pub fn handle(&self) -> io::Result<TerminalHandle> {
        let input = {
            let held = self
                .inner
                .input
                .lock()
                .map_err(|_| io::Error::other("poisoned"))?;
            duplicate_here(held.0)?
        };
        Ok(TerminalHandle {
            input,
            output: duplicate_here(self.output.0)?,
        })
    }

    /// Starts reading this terminal into a bounded buffer (ADR-0079 decision 4).
    pub fn start_reader(&self) -> io::Result<TerminalReader> {
        TerminalReader::start(Arc::clone(&self.output))
    }

    /// Reads what is waiting now into `buf`, never blocking. `true` at the end of the terminal (every write end of its output closed,
    /// which the pseudo-console does once its program ended). Only with no reader running.
    pub fn drain(&self, buf: &mut crate::held::HeldBuffer) -> io::Result<bool> {
        let mut chunk = vec![0u8; CHUNK];
        loop {
            let mut waiting = 0u32;
            // SAFETY: a live pipe handle; only the count of waiting bytes is asked for.
            let peeked = unsafe {
                PeekNamedPipe(
                    self.output.0,
                    null_mut(),
                    0,
                    null_mut(),
                    &mut waiting,
                    null_mut(),
                )
            };
            if peeked == 0 {
                return broken_is_end(unsafe { GetLastError() });
            }
            if waiting == 0 {
                return Ok(false);
            }
            let want = (waiting as usize).min(CHUNK) as u32;
            let mut n = 0u32;
            // SAFETY: `chunk` is valid for CHUNK bytes; at most what is waiting is read, so the call does not block.
            let ok = unsafe {
                ReadFile(
                    self.output.0,
                    chunk.as_mut_ptr().cast(),
                    want,
                    &mut n,
                    null_mut(),
                )
            };
            if ok == 0 {
                return broken_is_end(unsafe { GetLastError() });
            }
            buf.push(&chunk[..n as usize]);
        }
    }

    #[allow(dead_code)] // Types into the terminal without a node; the tests do.
    pub(crate) fn write_input(&self, bytes: &[u8]) -> io::Result<()> {
        self.inner.write_input(bytes)
    }
}

/// A pipe whose writers all closed is the terminal's end (`Ok(true)`); any other failure is an error.
fn broken_is_end(error: u32) -> io::Result<bool> {
    if error == ERROR_BROKEN_PIPE {
        Ok(true)
    } else {
        Err(io::Error::from_raw_os_error(error as i32))
    }
}

impl Drop for HeldTerminal {
    /// ADR-0079 decision 8: a terminal the holder lets go of ends with it, as a dropped [`spawn`] terminal does.
    fn drop(&mut self) {
        let _ = self.inner.kill();
    }
}

/// A held terminal read into a bounded buffer while no node is attached (ADR-0079 decision 4): the oldest bytes go first, so the agent
/// never blocks on a full pipe. [`Self::end`] cancels a read in flight and waits for the thread, so no byte is left in it.
pub struct TerminalReader {
    output: Arc<Handle>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    shared: Arc<Mutex<crate::held::Collected>>,
}

impl TerminalReader {
    fn start(output: Arc<Handle>) -> io::Result<Self> {
        use crate::held::{Collected, HeldBuffer};
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Mutex::new(Collected {
            buffer: HeldBuffer::default(),
            ended: false,
        }));
        let (pipe, halt, kept) = (Arc::clone(&output), Arc::clone(&stop), Arc::clone(&shared));
        let thread = std::thread::Builder::new()
            .name("agora-held-reader".into())
            .spawn(move || {
                let mut buf = vec![0u8; CHUNK];
                while !halt.load(Ordering::Acquire) {
                    let mut n = 0u32;
                    // SAFETY: `buf` is valid for CHUNK bytes; a synchronous pipe handle the Arc keeps open.
                    let ok = unsafe {
                        ReadFile(
                            pipe.0,
                            buf.as_mut_ptr().cast(),
                            CHUNK as u32,
                            &mut n,
                            null_mut(),
                        )
                    };
                    let error = if ok == 0 {
                        unsafe { GetLastError() }
                    } else {
                        0
                    };
                    let Ok(mut held) = kept.lock() else { break };
                    // Kept even when the read completed as it was being cancelled: those bytes are out of the pipe now.
                    held.buffer.push(&buf[..n as usize]);
                    if ok == 0 || n == 0 {
                        // A cancel (`end`) is not the terminal's end; anything else is (the pipe broken: its writers closed).
                        held.ended |= error != ERROR_OPERATION_ABORTED;
                        break;
                    }
                }
            })?;
        Ok(Self {
            output,
            stop,
            thread: Some(thread),
            shared,
        })
    }

    /// Whether the reader saw the terminal's end.
    pub fn ended(&self) -> bool {
        self.shared.lock().map(|c| c.ended).unwrap_or(true)
    }

    /// How many bytes it holds.
    pub fn buffered(&self) -> usize {
        self.shared.lock().map(|c| c.buffer.len()).unwrap_or(0)
    }

    /// Ends the read (waiting for it, so no byte is in flight) and gives what it read.
    pub fn end(mut self) -> crate::held::Collected {
        self.halt();
        let mut kept = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::held::Collected {
            buffer: std::mem::take(&mut kept.buffer),
            ended: kept.ended,
        }
    }

    fn halt(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.stop.store(true, Ordering::Release);
            // The thread may be between two reads when the first cancel comes: cancel until it has ended.
            while !thread.is_finished() {
                // SAFETY: a live pipe handle; cancels the reads in flight on it, from any thread.
                unsafe { CancelIoEx(self.output.0, null()) };
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let _ = thread.join();
        }
    }
}

impl Drop for TerminalReader {
    fn drop(&mut self) {
        self.halt();
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

/// M2.6: the holder's terminal in the shape the holder uses on every system (`agora hold`): no reading until a reader starts, a reader
/// whose end leaves nothing in flight, `drain` for an ended agent, the signals, and the passable pipe ends.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod held_tests {
    use super::*;
    use crate::held::{HeldBuffer, HeldSignal};
    use std::time::{Duration, Instant};

    fn config(argv: &[&str]) -> PtyConfig {
        PtyConfig::new(argv.iter().map(|a| (*a).to_string()).collect())
    }

    /// Waits up to 20 s for the agent's end.
    fn ended(term: &mut HeldTerminal) -> Option<ExitStatus> {
        let end = Instant::now() + Duration::from_secs(20);
        while Instant::now() < end {
            if let Some(s) = term.try_wait().unwrap() {
                return Some(s);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }

    #[test]
    fn a_reader_keeps_the_output_and_its_end_leaves_nothing_in_flight() {
        let term = HeldTerminal::start(&config(&["cmd.exe"])).unwrap();
        let mut seen = String::new();
        let end = Instant::now() + Duration::from_secs(20);
        let mut typed = false;
        while Instant::now() < end && !seen.contains("one-Windows_NT") {
            let reader = term.start_reader().unwrap();
            if !typed {
                term.write_input(b"echo one-%OS%\r\n").unwrap();
                typed = true;
            }
            std::thread::sleep(Duration::from_millis(100));
            let c = reader.end();
            assert!(!c.ended, "a cancelled read is not the terminal's end");
            seen.push_str(&String::from_utf8_lossy(&c.buffer.into_vec()));
        }
        assert!(seen.contains("one-Windows_NT"), "{seen:?}");
    }

    #[test]
    fn a_reader_sees_the_end_of_an_agent_that_ends() {
        let mut term = HeldTerminal::start(&config(&["cmd.exe", "/c", "echo last-%OS%"])).unwrap();
        let reader = term.start_reader().unwrap();
        assert!(ended(&mut term).is_some());
        let end = Instant::now() + Duration::from_secs(20);
        while !reader.ended() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(reader.ended());
        assert!(reader.buffered() > 0);
        let c = reader.end();
        assert!(c.ended);
        let text = String::from_utf8_lossy(&c.buffer.into_vec()).into_owned();
        assert!(text.contains("last-Windows_NT"), "{text:?}");
    }

    #[test]
    fn drain_takes_what_an_ended_agent_left_without_blocking() {
        let mut term = HeldTerminal::start(&config(&["cmd.exe", "/c", "echo left-%OS%"])).unwrap();
        assert_eq!(ended(&mut term).map(|s| s.code), Some(Some(0)));
        let mut buf = HeldBuffer::default();
        let end = Instant::now() + Duration::from_secs(20);
        let mut eof = false;
        while !eof && Instant::now() < end {
            let before = Instant::now();
            eof = term.drain(&mut buf).unwrap();
            assert!(before.elapsed() < Duration::from_secs(2), "drain blocked");
            if !eof {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        assert!(eof, "the end of the terminal was not seen");
        let text = String::from_utf8_lossy(&buf.into_vec()).into_owned();
        assert!(text.contains("left-Windows_NT"), "{text:?}");
    }

    #[test]
    fn drain_of_a_running_quiet_agent_returns_at_once() {
        let mut term = HeldTerminal::start(&config(&["ping", "-n", "30", "127.0.0.1"])).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let mut buf = HeldBuffer::default();
        let before = Instant::now();
        assert!(!term.drain(&mut buf).unwrap());
        assert!(before.elapsed() < Duration::from_secs(1));
        term.kill_and_reap();
        assert!(term.try_wait().unwrap().is_some());
    }

    /// A detached start (`CREATE_NEW_PROCESS_GROUP`) turns Ctrl-C off for the starter and what it starts; `Interrupt` must reach the
    /// agent anyway. The test turns it off for its own process first, as such a start would have.
    #[test]
    fn interrupt_reaches_the_agent_even_from_a_starter_that_ignores_ctrl_c() {
        // SAFETY: plain call; ignore Ctrl-C in this process, as a process in a new process group does.
        unsafe { SetConsoleCtrlHandler(None, 1) };
        let mut term = HeldTerminal::start(&config(&["cmd.exe"])).unwrap();
        // Reads with a holder's reader, ended and started again, until `needle` was read or 20 s passed.
        let read_until = |seen: &mut String, needle: &str| {
            let end = Instant::now() + Duration::from_secs(20);
            while Instant::now() < end && !seen.contains(needle) {
                let reader = term.start_reader().unwrap();
                std::thread::sleep(Duration::from_millis(100));
                seen.push_str(&String::from_utf8_lossy(&reader.end().buffer.into_vec()));
            }
        };
        let mut seen = String::new();
        read_until(&mut seen, ">");
        term.write_input(b"ping -t 127.0.0.1\r\n").unwrap();
        read_until(&mut seen, "Reply from");
        assert!(seen.contains("Reply from"), "ping did not start: {seen:?}");
        term.signal(HeldSignal::Interrupt).unwrap();
        read_until(&mut seen, "Control-C");
        assert!(seen.contains("Control-C"), "{seen:?}");
        let end = Instant::now() + Duration::from_secs(10);
        while !crate::process_children(term.pid())
            .unwrap_or_default()
            .is_empty()
            && Instant::now() < end
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(crate::process_children(term.pid())
            .unwrap_or_default()
            .is_empty());
        term.kill_and_reap();
    }

    #[test]
    fn each_signal_ends_the_program_and_none_reaches_an_ended_one() {
        for signal in [
            HeldSignal::Interrupt,
            HeldSignal::Terminate,
            HeldSignal::Kill,
        ] {
            let mut term = HeldTerminal::start(&config(&["ping", "-t", "127.0.0.1"])).unwrap();
            // The pseudo-console takes a moment before Ctrl-C reaches the program in it.
            let reader = term.start_reader().unwrap();
            let wait = Instant::now() + Duration::from_secs(10);
            while reader.buffered() == 0 && Instant::now() < wait {
                std::thread::sleep(Duration::from_millis(20));
            }
            term.signal(signal).unwrap();
            assert!(ended(&mut term).is_some(), "{signal:?} did not end it");
            drop(reader);
            // Ended: a signal is nothing now.
            term.signal(HeldSignal::Kill).unwrap();
        }
    }

    #[test]
    fn the_passable_ends_read_the_terminal_as_a_file() {
        use std::io::Read;
        let mut term = HeldTerminal::start(&config(&["cmd.exe", "/c", "echo file-%OS%"])).unwrap();
        let mut file = term.handle().unwrap().into_file().unwrap();
        assert!(ended(&mut term).is_some());
        let mut text = Vec::new();
        file.read_to_end(&mut text).unwrap();
        assert!(String::from_utf8_lossy(&text).contains("file-Windows_NT"));
    }
}

/// M2.6: a held terminal handed to the node inside a frame: the reference, its import, and the duplicates closed again when the frame
/// never went out. The "node" is this test process (opened and checked by its start time like any peer).
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod reference_tests {
    use super::*;
    use std::io::Read;
    use std::time::{Duration, Instant};

    fn me() -> crate::ProcessWatch {
        let pid = std::process::id();
        crate::ProcessWatch::open(pid, crate::process_start_time(pid).unwrap()).unwrap()
    }

    fn values(reference: &[u8]) -> [HANDLE; 2] {
        let at = |i: usize| {
            let mut raw = [0u8; 8];
            raw.copy_from_slice(&reference[i..i + 8]);
            u64::from_be_bytes(raw) as usize as HANDLE
        };
        [at(1), at(9)]
    }

    /// Whether `value` (a handle of this process) still names the same object as `original`.
    fn same_object(value: HANDLE, original: HANDLE) -> bool {
        // SAFETY: CompareObjectHandles only compares; a closed or reused value gives FALSE for "not the same".
        unsafe { windows_sys::Win32::Foundation::CompareObjectHandles(value, original) != 0 }
    }

    #[test]
    fn a_reference_imports_as_the_same_terminal() {
        let peer = me();
        let mut term = HeldTerminal::start(&PtyConfig::new(vec![
            "cmd.exe".into(),
            "/c".into(),
            "echo ref-%OS%".into(),
        ]))
        .unwrap();
        let handle = term.handle().unwrap();
        let export = handle.export_to(&peer).unwrap();
        assert_eq!(export.bytes().len(), TERMINAL_REF_LEN);
        assert!(export.bytes().len() <= crate::TERMINAL_REF_MAX);
        let imported = TerminalHandle::import(export.bytes()).unwrap();
        export.delivered();
        let mut file = imported.into_file().unwrap();
        let end = Instant::now() + Duration::from_secs(20);
        while term.try_wait().unwrap().is_none() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut text = Vec::new();
        file.read_to_end(&mut text).unwrap();
        assert!(String::from_utf8_lossy(&text).contains("ref-Windows_NT"));
    }

    #[test]
    fn an_export_that_was_not_delivered_closes_its_duplicates_in_the_node() {
        let peer = me();
        let term = HeldTerminal::start(&PtyConfig::new(vec!["cmd.exe".into()])).unwrap();
        let handle = term.handle().unwrap();
        let export = handle.export_to(&peer).unwrap();
        let [input, output] = values(export.bytes());
        assert!(same_object(input, handle.input_raw()));
        assert!(same_object(output, handle.output_raw()));
        // The frame never went out (a send failed, the link dropped): nothing is left in the node.
        drop(export);
        assert!(!same_object(input, handle.input_raw()));
        assert!(!same_object(output, handle.output_raw()));
    }

    #[test]
    fn a_delivered_export_leaves_the_duplicates_to_the_node() {
        let peer = me();
        let term = HeldTerminal::start(&PtyConfig::new(vec!["cmd.exe".into()])).unwrap();
        let handle = term.handle().unwrap();
        let export = handle.export_to(&peer).unwrap();
        let bytes = export.bytes().to_vec();
        export.delivered();
        let [input, output] = values(&bytes);
        assert!(same_object(input, handle.input_raw()));
        assert!(same_object(output, handle.output_raw()));
        // The node owns them: importing and dropping is how it closes them.
        drop(TerminalHandle::import(&bytes).unwrap());
        assert!(!same_object(input, handle.input_raw()));
    }

    #[test]
    fn a_bad_reference_is_refused_and_closes_nothing() {
        let file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        let file_value = {
            use std::os::windows::io::AsRawHandle;
            file.as_raw_handle() as usize as u64
        };
        let term = HeldTerminal::start(&PtyConfig::new(vec!["cmd.exe".into()])).unwrap();
        let handle = term.handle().unwrap();
        let pipe_value = handle.output_raw() as usize as u64;
        let reference = |version: u8, a: u64, b: u64| {
            let mut r = vec![version];
            r.extend_from_slice(&a.to_be_bytes());
            r.extend_from_slice(&b.to_be_bytes());
            r
        };
        let refused = |r: &[u8]| {
            assert_eq!(
                TerminalHandle::import(r).err().map(|e| e.kind()),
                Some(io::ErrorKind::InvalidData),
                "{r:?}"
            )
        };
        refused(&[]);
        refused(&reference(1, pipe_value, pipe_value)[..16]);
        refused(&[reference(1, pipe_value, pipe_value), vec![0]].concat());
        refused(&reference(
            2,
            pipe_value,
            handle.input_raw() as usize as u64,
        ));
        refused(&reference(1, 0, pipe_value));
        refused(&reference(1, u64::MAX, pipe_value));
        // The same handle twice, and a handle that is no pipe (a file this test holds open).
        refused(&reference(1, pipe_value, pipe_value));
        refused(&reference(1, file_value, pipe_value));
        // Nothing of a refused reference was closed: the file and the pipe end still work.
        assert!(file.metadata().is_ok());
        // SAFETY: only reads the type of a handle this test owns.
        assert_eq!(unsafe { GetFileType(handle.output_raw()) }, FILE_TYPE_PIPE);
    }
}
