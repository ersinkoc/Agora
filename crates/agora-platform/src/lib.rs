//! The only crate allowed to contain OS-specific code: PTY/ConPTY, process tree, cwd, local sockets and pipes, resource limits.
//!
//! The crate root exposes platform-neutral types and traits; each OS implements them in `src/<os>.rs`.
//! No `#[cfg(target_os)]` or `#[cfg(unix)]` is allowed outside this crate (AGENTS.md).

use std::io;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

#[cfg(windows)]
mod conpty;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod winconfine;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod winproc;
#[cfg(windows)]
use winproc as procinfo;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
use linux as procinfo;
#[cfg(target_os = "macos")]
use macos as procinfo;

/// True where [`local_listen`] and [`local_connect`] work (Unix domain sockets; Windows named pipes, ADR-0045). Callers and tests
/// use this instead of any `cfg`.
pub const LOCAL_SOCKETS_SUPPORTED: bool = cfg!(any(unix, windows));

/// True where [`spawn`] can start a PTY.
pub const PTY_SUPPORTED: bool = cfg!(any(unix, windows));

/// A connected local (same-machine) stream, used for the node's control socket.
pub struct LocalStream {
    #[cfg(unix)]
    inner: std::os::unix::net::UnixStream,
    #[cfg(windows)]
    inner: windows::Stream,
}

/// A listening local socket. Removes its socket file when dropped.
pub struct LocalListener {
    #[cfg(unix)]
    inner: std::os::unix::net::UnixListener,
    #[cfg(unix)]
    path: PathBuf,
    #[cfg(windows)]
    inner: windows::Listener,
}

#[cfg(not(unix))]
fn unsupported_io(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{what} is not implemented on this platform yet"),
    )
}

/// Listen on a local socket at `path`, readable and writable only by the current user. A stale socket file
/// left by a crashed process is replaced; one that another process still serves is an error
/// (`AddrInUse`).
pub fn local_listen(path: &std::path::Path) -> io::Result<LocalListener> {
    #[cfg(unix)]
    {
        unix::local_listen(path)
    }
    #[cfg(windows)]
    {
        Ok(LocalListener {
            inner: windows::listen(path)?,
        })
    }
}

/// A fresh identity for one agent that is about to be confined (ADR-0050 Amendment B on Windows: a random SID, used both for its
/// process and its mesh endpoint), or `None` where agents are not confined that way or confinement is off. Make it before the
/// agent's endpoint, and pass it to [`local_listen_agent`] and [`PtyConfig::identity`].
pub fn new_agent_identity() -> Option<String> {
    #[cfg(windows)]
    if winconfine::enabled() {
        return winconfine::random_sid_string().ok();
    }
    None
}

/// Like [`local_listen`], for one agent's own channel (its mesh endpoint). With an `identity` from [`new_agent_identity`] (Windows,
/// ADR-0050 Amendment B), the confined agent holding exactly that identity may connect too, and no other confined agent can: the
/// mesh binds an agent's identity to the connection it comes in on. Without one, or on other systems, the same as [`local_listen`].
pub fn local_listen_agent(
    path: &std::path::Path,
    identity: Option<&str>,
) -> io::Result<LocalListener> {
    #[cfg(unix)]
    {
        let _ = identity;
        unix::local_listen(path)
    }
    #[cfg(windows)]
    {
        Ok(LocalListener {
            inner: windows::listen_agent(path, identity)?,
        })
    }
}

/// The system name of the endpoint [`local_listen`] serves for `path` (`\\.\pipe\...` on Windows), for tests and diagnostics; `None`
/// where the endpoint is the path itself (Unix sockets).
pub fn local_endpoint_name(path: &std::path::Path) -> Option<String> {
    #[cfg(windows)]
    {
        Some(windows::pipe_name_text(path))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

/// Puts a freshly downloaded program in place of the one at `target` (which may be running): the new file is made executable and
/// renamed over it. Windows cannot replace a running program, so the old one is moved aside to `<target>.old` first.
pub fn replace_executable(new: &std::path::Path, target: &std::path::Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(new, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(new, target)
    }
    #[cfg(not(unix))]
    {
        let mut old = target.as_os_str().to_owned();
        old.push(".old");
        let old = std::path::PathBuf::from(old);
        let _ = std::fs::remove_file(&old);
        std::fs::rename(target, &old)?;
        std::fs::rename(new, target).inspect_err(|_| {
            let _ = std::fs::rename(&old, target);
        })
    }
}

/// Sets or clears the execute bits of an unpacked file (`agora adapter install`). Elsewhere than Unix a file runs by its name, so this does nothing.
pub fn set_executable(path: &std::path::Path, executable: bool) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if executable { 0o700 } else { 0o600 };
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, executable);
        Ok(())
    }
}

/// Makes a folder usable by the current user only (the folder of installed adapters): `0700` on Unix; on Windows a protected ACL for
/// the user and SYSTEM, passed on to everything below. A no-op elsewhere.
pub fn restrict_to_owner(path: &std::path::Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
    }
    #[cfg(windows)]
    {
        windows::set_owner_only(path, true)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Ok(())
    }
}

/// Writes `contents` to `path` readable and writable only by the current user (private keys and tokens), replacing any
/// existing file: `0600` on Unix; on Windows a protected ACL for the user and SYSTEM, so a folder that lets other users in
/// (a data volume's default ACL) does not reach the file.
pub fn write_private(path: &std::path::Path, contents: &[u8]) -> io::Result<()> {
    #[cfg(any(unix, windows))]
    {
        use std::io::Write;
        // Written beside the target and renamed over it: a full disk or an interruption leaves the old file whole, never
        // a half-written key or token. The temporary name is exclusive (`create_new`), so a planted link is never followed.
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(format!(".tmp-{}", std::process::id()));
        let tmp = std::path::PathBuf::from(tmp);
        let _ = std::fs::remove_file(&tmp);
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut f = options.open(&tmp)?;
            // Private before the first byte is in it.
            #[cfg(windows)]
            windows::set_owner_only(&tmp, false)?;
            f.write_all(contents)?;
            f.sync_all()?;
            // Windows cannot rename a file that is still open.
            drop(f);
            std::fs::rename(&tmp, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }
    #[cfg(not(any(unix, windows)))]
    {
        std::fs::write(path, contents)
    }
}

/// `std::fs::canonicalize`, spelled the way programs expect: on Windows without the verbatim `\\?\` prefix whenever the ordinary
/// spelling names the same file (`cmd.exe` cannot start in a verbatim folder and silently uses `C:\Windows` instead). Use it for any
/// path a process is started in or that is shown to people. Elsewhere it is `std::fs::canonicalize`.
pub fn canonicalize_plain(path: &std::path::Path) -> io::Result<std::path::PathBuf> {
    let canonical = std::fs::canonicalize(path)?;
    #[cfg(windows)]
    {
        Ok(windows::plain_path(canonical))
    }
    #[cfg(not(windows))]
    {
        Ok(canonical)
    }
}

/// Creates a symbolic link at `link` pointing to `target`. Only Unix for now (Windows links need privileges and a file
/// or directory flavour); elsewhere it returns `Unsupported`, which tests that need a link use to skip themselves.
pub fn symlink(target: &std::path::Path, link: &std::path::Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(not(unix))]
    {
        let _ = (target, link);
        Err(unsupported_io("symbolic links"))
    }
}

/// Connect to a local socket.
pub fn local_connect(path: &std::path::Path) -> io::Result<LocalStream> {
    #[cfg(unix)]
    {
        unix::local_connect(path)
    }
    #[cfg(windows)]
    {
        Ok(LocalStream {
            inner: windows::connect(path)?,
        })
    }
}

impl LocalListener {
    /// Wait for the next client.
    pub fn accept(&self) -> io::Result<LocalStream> {
        #[cfg(unix)]
        {
            self.inner.accept().map(|(inner, _)| LocalStream { inner })
        }
        #[cfg(windows)]
        {
            self.inner.accept().map(|inner| LocalStream { inner })
        }
    }

    /// Make [`accept`](Self::accept) return `WouldBlock` instead of waiting.
    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        #[cfg(unix)]
        {
            self.inner.set_nonblocking(nonblocking)
        }
        #[cfg(windows)]
        {
            self.inner.set_nonblocking(nonblocking);
            Ok(())
        }
    }
}

#[cfg(unix)]
impl Drop for LocalListener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl LocalStream {
    pub fn try_clone(&self) -> io::Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                inner: self.inner.try_clone()?,
            })
        }
        #[cfg(windows)]
        {
            Ok(Self {
                inner: self.inner.try_clone()?,
            })
        }
    }

    pub fn set_read_timeout(&self, timeout: Option<std::time::Duration>) -> io::Result<()> {
        #[cfg(unix)]
        {
            self.inner.set_read_timeout(timeout)
        }
        #[cfg(windows)]
        {
            self.inner.set_read_timeout(timeout)
        }
    }

    /// Close both directions, waking a thread blocked reading this stream.
    pub fn shutdown(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            self.inner.shutdown(std::net::Shutdown::Both)
        }
        #[cfg(windows)]
        {
            self.inner.shutdown()
        }
    }
}

impl io::Read for LocalStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        #[cfg(unix)]
        {
            io::Read::read(&mut self.inner, buf)
        }
        #[cfg(windows)]
        {
            self.inner.read(buf)
        }
    }
}

impl io::Write for LocalStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        #[cfg(unix)]
        {
            io::Write::write(&mut self.inner, buf)
        }
        #[cfg(windows)]
        {
            self.inner.write(buf)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        {
            io::Write::flush(&mut self.inner)
        }
        #[cfg(windows)]
        {
            Ok(())
        }
    }
}

/// Puts the controlling terminal (stdin) into raw mode and restores it when dropped. For `terminal attach`.
pub struct RawMode {
    #[cfg(unix)]
    saved: libc::termios,
    #[cfg(windows)]
    saved_in: u32,
    #[cfg(windows)]
    saved_out: Option<u32>,
}

impl RawMode {
    /// Fails when stdin is not a terminal.
    pub fn enable() -> io::Result<Self> {
        #[cfg(unix)]
        {
            unix::enable_raw_mode()
        }
        #[cfg(windows)]
        {
            windows::enable_raw_mode()
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        // Restores the saved terminal settings.
        #[cfg(unix)]
        unix::restore_mode(&self.saved);
        #[cfg(windows)]
        windows::restore_mode(self);
    }
}

/// Size of the terminal on stdout as `(cols, rows)`, or `None` when stdout is not a terminal.
pub fn terminal_size() -> Option<(u16, u16)> {
    #[cfg(unix)]
    {
        unix::terminal_size()
    }
    #[cfg(windows)]
    {
        windows::terminal_size()
    }
}

/// Per-process OS limits, applied to the child before it starts. `None` means unlimited.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcessLimits {
    /// CPU time in seconds (`RLIMIT_CPU`); the process is killed by the OS when it is exhausted.
    pub cpu_seconds: Option<u64>,
    /// Address space in bytes (`RLIMIT_AS`). Enforced on Linux only: macOS ignores it, so it is not applied
    /// there. Runtimes such as Node reserve large virtual ranges, so leave this unset unless needed.
    pub memory_bytes: Option<u64>,
}

/// How to start a process in a new pseudo-terminal.
#[derive(Debug, Clone)]
pub struct PtyConfig {
    /// Program and arguments; must not be empty.
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Extra environment variables. `TERM` defaults to `xterm-256color` unless set here.
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
    pub limits: ProcessLimits,
    /// Confine the process to these paths (ADR-0047). `spawn` refuses, rather than ignores it, where this cannot be enforced.
    pub sandbox: Option<Sandbox>,
    /// The agent's own identity under confinement, from [`new_agent_identity`]: the confined process runs with it, and its mesh endpoint
    /// ([`local_listen_agent`]) admits exactly it. `None` makes a fresh one (or means nothing where agents are not confined this way).
    pub identity: Option<String>,
}

/// File-system confinement for a process and everything it starts: it may read (and run programs from) `read` and `write`, and
/// create, change and remove files only below `write`. Everything else is denied. Paths that do not exist on this machine are skipped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sandbox {
    pub read: Vec<PathBuf>,
    pub write: Vec<PathBuf>,
    /// No network but the loopback interface and the unix sockets below `sockets` and `write` (ADR-0047 `worktree-offline`).
    /// Refused where [`offline_supported`] is false.
    pub offline: bool,
    /// Folders whose unix sockets stay reachable when `offline` (the node's mesh sockets).
    pub sockets: Vec<PathBuf>,
}

impl Sandbox {
    /// Files only: read `read`, write below `write`; the network is not restricted.
    pub fn new(read: Vec<PathBuf>, write: Vec<PathBuf>) -> Self {
        Self {
            read,
            write,
            ..Self::default()
        }
    }
}

/// True where a [`Sandbox`] is enforced by per-agent folder permissions (Windows, ADR-0050) rather than by a rule list the kernel
/// reads. There the node hands an agent only its worktree, its own Git admin folder, the shared `objects/` and `refs/`, and a private
/// temp folder, and leaves the rest of the shared Git folder (config, hooks) read-only. Callers use this instead of a `cfg`.
pub const NARROW_GIT_WRITES: bool = cfg!(windows);

/// True where a confined agent may write only the parts of its checkout's shared Git folder a commit needs (`objects`, `refs`,
/// `logs/refs`, its own worktree admin folder; on macOS also `packed-refs`) and not `config` or `hooks`: a planted config or hook would
/// run in the user's own git and the node's. Windows (narrow profile) and macOS; Linux still gives the whole shared folder. Callers use
/// this instead of a `cfg`.
pub const GIT_CONFIG_READ_ONLY: bool = cfg!(any(windows, target_os = "macos"));

/// True where a confined agent gets a temp folder of its own (`TMPDIR`, `TEMP`, `TMP` point at it): on macOS the per-user temp folder
/// is private to the sandbox and on Windows the profile's is out of reach, so without one compilers and `mktemp` fail. On Linux the
/// shared `/tmp` is writable to the agent instead. Callers use this instead of a `cfg`.
pub const PRIVATE_TEMP: bool = cfg!(any(windows, target_os = "macos"));

/// True when this machine can enforce a [`Sandbox`] (Linux with Landlock). `agora doctor` and the node ask before starting anything.
pub fn sandbox_supported() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::sandbox_supported()
    }
    #[cfg(target_os = "macos")]
    {
        macos::sandbox_supported()
    }
    #[cfg(windows)]
    {
        winconfine::enabled()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        false
    }
}

/// True when this machine can also keep a confined process off the network ([`Sandbox::offline`]): macOS (Seatbelt, loopback and
/// the listed unix sockets stay reachable). Linux (Landlock network rules) and Windows do not yet, so `worktree-offline` is refused
/// there.
pub fn offline_supported() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::sandbox_supported()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Whether `sb` can be enforced here as asked: confinement at all, and the network part when it asks for it.
fn enforceable(sb: &Sandbox) -> bool {
    sandbox_supported() && (!sb.offline || offline_supported())
}

/// Why a confined start was refused before the agent ran (ADR-0050 Amendment B). Carried inside the `PermissionDenied` error that
/// [`spawn`] returns, read back with [`sandbox_refusal`]; `token` is the stable `reason` parameter of `agent.sandbox_unavailable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxRefusal {
    /// A folder the agent would be given (or something below it) lets `Users` or `Everyone` write.
    BroadWriteGrant,
    /// A folder the agent would write could not be labelled for confined agents.
    LabelFailed,
}

impl SandboxRefusal {
    pub fn token(self) -> &'static str {
        match self {
            Self::BroadWriteGrant => "broad_write_grant",
            Self::LabelFailed => "label_failed",
        }
    }
}

impl std::fmt::Display for SandboxRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.token())
    }
}

impl std::error::Error for SandboxRefusal {}

/// The refusal inside an error from [`spawn`], if that is what it was.
pub fn sandbox_refusal(e: &io::Error) -> Option<SandboxRefusal> {
    e.get_ref()
        .and_then(|inner| inner.downcast_ref::<SandboxRefusal>())
        .copied()
}

/// Checks, before anything starts, that `sandbox` can be enforced as promised (ADR-0050 Amendment B, 4): on Windows a writable folder,
/// or anything below it, that lets `Users` or `Everyone` write would let every confined agent in, so it is refused and the first such
/// path is returned. Always `Ok` elsewhere, and on Windows while confinement is off.
pub fn sandbox_preflight(sandbox: &Sandbox) -> Result<(), PathBuf> {
    #[cfg(windows)]
    if winconfine::enabled() {
        return winconfine::preflight(sandbox).map_err(|(path, _)| path);
    }
    let _ = sandbox;
    Ok(())
}

/// Undoes the folder permissions a crashed node left behind for a confined agent (Windows, ADR-0050). Nothing to do elsewhere.
/// Call it once when a node starts.
pub fn recover_sandbox_leftovers() {
    // In the background: undoing a permission walks every file below the folder, which can take minutes, and the node must not wait
    // for it. A ledger of a node that is still running is left alone.
    #[cfg(windows)]
    let _ = std::thread::Builder::new()
        .name("agora-acl-recovery".into())
        .spawn(winconfine::recover_stale);
}

/// Confines `cmd` (and what it starts) the same way [`PtyConfig::sandbox`] does a PTY process. Fails with `Unsupported` where that
/// cannot be enforced, so the caller never runs the command unconfined by accident.
pub fn confine_command(cmd: &mut std::process::Command, sandbox: &Sandbox) -> io::Result<()> {
    if !enforceable(sandbox) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this machine cannot confine a process",
        ));
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let ruleset = linux::ruleset(sandbox)?;
        // SAFETY: the closure only applies a ruleset made before the fork (async-signal-safe calls).
        unsafe { cmd.pre_exec(move || ruleset.enforce()) };
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        // The command becomes `sandbox-exec -p <profile> -- <program> <args>`. Environment and folder carry over; standard streams do
        // not (std cannot read them back), so callers confine first and set the streams after.
        let (program, args) = {
            let a: Vec<_> = cmd.get_args().map(ToOwned::to_owned).collect();
            macos::wrap(sandbox, cmd.get_program(), &a)
        };
        let mut wrapped = std::process::Command::new(program);
        wrapped.args(args);
        if let Some(dir) = cmd.get_current_dir() {
            wrapped.current_dir(dir);
        }
        for (k, v) in cmd.get_envs() {
            match v {
                Some(v) => wrapped.env(k, v),
                None => wrapped.env_remove(k),
            };
        }
        *cmd = wrapped;
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (cmd, sandbox);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no confinement here",
        ))
    }
}

impl PtyConfig {
    pub fn new(argv: Vec<String>) -> Self {
        Self {
            argv,
            cwd: None,
            env: Vec::new(),
            cols: 80,
            rows: 24,
            limits: ProcessLimits::default(),
            sandbox: None,
            identity: None,
        }
    }
}

/// How a process ended. Exactly one of the fields is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    /// Exit code when the process exited normally.
    pub code: Option<i32>,
    /// Signal number when it was terminated by a signal (Unix).
    pub signal: Option<i32>,
}

impl ExitStatus {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Operations on a running PTY that do not need exclusive access to it, so they can be used from any thread
/// while another thread reads its output. Input never waits behind output processing, and `kill` never waits
/// behind a blocked input write (ADR-0013: input outranks output).
#[derive(Clone)]
pub struct PtyControl {
    inner: std::sync::Arc<dyn PtyControlOps>,
}

pub(crate) trait PtyControlOps: Send + Sync {
    fn write_input(&self, bytes: &[u8]) -> io::Result<()>;
    fn resize(&self, cols: u16, rows: u16) -> io::Result<()>;
    fn kill(&self) -> io::Result<()>;
    fn foreground_pid(&self) -> io::Result<u32>;
    fn pid(&self) -> u32;
}

impl PtyControl {
    /// Send input to the process. Blocks while the process is not reading and the terminal's input queue is
    /// full; `resize` and `kill` are unaffected.
    pub fn write_input(&self, bytes: &[u8]) -> io::Result<()> {
        self.inner.write_input(bytes)
    }
    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        self.inner.resize(cols, rows)
    }
    /// Kill the child and its process group. A no-op once the child has been reaped.
    pub fn kill(&self) -> io::Result<()> {
        self.inner.kill()
    }
    pub fn foreground_pid(&self) -> io::Result<u32> {
        self.inner.foreground_pid()
    }
    pub fn pid(&self) -> u32 {
        self.inner.pid()
    }
}

/// A process running in a PTY.
///
/// Output is read on a dedicated OS thread and delivered through a bounded channel in chunks of at most
/// 16 KiB, so a busy terminal cannot starve the async runtime or queue ahead of input (ADR-0013). The channel
/// closes when the process side of the terminal is gone.
pub trait Pty: Send {
    /// Bytes the process wrote to the terminal.
    fn output(&self) -> &Receiver<Vec<u8>>;
    /// Send input to the process.
    fn write_input(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn resize(&mut self, cols: u16, rows: u16) -> io::Result<()>;
    /// Process id of the child (also its process group id).
    fn pid(&self) -> u32;
    /// Kill the child and everything in its process group.
    fn kill(&mut self) -> io::Result<()>;
    /// Non-blocking: `Some` once the child has exited.
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>>;
    /// Block until the child exits.
    fn wait(&mut self) -> io::Result<ExitStatus>;
    /// Process id of the terminal's foreground process group leader (what the user sees running).
    fn foreground_pid(&self) -> io::Result<u32>;
    /// A cloneable handle for input, resize and kill that works while another thread owns this `Pty` and
    /// reads its output.
    fn control(&self) -> io::Result<PtyControl>;
}

/// One process in a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    /// Executable name (the kernel's short name; may be truncated).
    pub name: String,
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "process inspection is not implemented on this platform",
    )
}

/// Short executable name of a process.
pub fn process_name(pid: u32) -> io::Result<String> {
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    {
        procinfo::name(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = pid;
        Err(unsupported())
    }
}

/// Current working directory of a process.
pub fn process_cwd(pid: u32) -> io::Result<PathBuf> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        procinfo::cwd(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        Err(unsupported())
    }
}

/// Memory and CPU use of a process, for footprint measurement and resource reporting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcessStats {
    /// Resident set size in bytes.
    pub rss_bytes: u64,
    /// CPU time used so far (user + system) in seconds.
    pub cpu_seconds: f64,
}

/// Current memory and CPU use of process `pid` (use `std::process::id()` for this process).
pub fn process_stats(pid: u32) -> io::Result<ProcessStats> {
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    {
        procinfo::stats(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = pid;
        Err(unsupported())
    }
}

/// Direct children of a process.
pub fn process_children(pid: u32) -> io::Result<Vec<u32>> {
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    {
        procinfo::children(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = pid;
        Err(unsupported())
    }
}

/// The process and all its descendants, parents before children. Processes that exit while the tree is
/// being walked are skipped.
pub fn process_tree(root: u32) -> io::Result<Vec<ProcessInfo>> {
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from([(root, 0u32)]);
    while let Some((pid, ppid)) = queue.pop_front() {
        let Ok(name) = process_name(pid) else {
            if pid == root {
                return Err(io::Error::new(io::ErrorKind::NotFound, "process not found"));
            }
            continue;
        };
        out.push(ProcessInfo { pid, ppid, name });
        if let Ok(kids) = process_children(pid) {
            queue.extend(kids.into_iter().map(|k| (k, pid)));
        }
    }
    Ok(out)
}

/// Start a process in a new PTY.
pub fn spawn(config: &PtyConfig) -> io::Result<Box<dyn Pty>> {
    if config.argv.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "argv is empty"));
    }
    if config.sandbox.as_ref().is_some_and(|sb| !enforceable(sb)) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this machine cannot confine a process",
        ));
    }
    #[cfg(unix)]
    {
        unix::spawn(config)
    }
    #[cfg(windows)]
    {
        conpty::spawn(config)
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod procinfo_tests {
    use super::*;

    #[test]
    fn own_process_stats_are_plausible_and_cpu_time_grows() {
        let a = process_stats(std::process::id()).expect("stats");
        assert!(a.rss_bytes > 100_000, "{a:?}");
        // Burn a little CPU; cpu_seconds must not go backwards (and normally goes up).
        let mut x = 0u64;
        let t = std::time::Instant::now();
        while t.elapsed() < Duration::from_millis(300) {
            x = x.wrapping_mul(31).wrapping_add(1);
        }
        std::hint::black_box(x);
        let b = process_stats(std::process::id()).expect("stats");
        assert!(b.cpu_seconds >= a.cpu_seconds, "{a:?} {b:?}");
        assert!(process_stats(u32::MAX - 1).is_err());
    }
    use std::time::{Duration, Instant};

    fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if f() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("timed out waiting for {what}");
    }

    #[test]
    fn reports_cwd_foreground_and_tree() {
        let mut p = spawn(&PtyConfig::new(vec!["sh".into()])).expect("spawn");
        p.write_input(b"cd / && sleep 30\n").expect("write");
        let shell = p.pid();

        wait_until("sleep in the foreground", || {
            p.foreground_pid()
                .ok()
                .and_then(|pid| process_name(pid).ok())
                .is_some_and(|n| n == "sleep")
        });
        let fg = p.foreground_pid().expect("fg");
        assert_ne!(fg, shell);
        assert_eq!(process_cwd(fg).expect("cwd"), PathBuf::from("/"));

        let tree = process_tree(shell).expect("tree");
        assert_eq!(tree[0].pid, shell);
        assert!(tree
            .iter()
            .any(|i| i.pid == fg && i.name == "sleep" && i.ppid == shell));

        p.kill().expect("kill");
        p.wait().expect("wait");
    }

    #[test]
    fn missing_process_is_an_error() {
        assert!(process_name(u32::MAX - 1).is_err());
        assert!(process_tree(u32::MAX - 1).is_err());
    }
}
