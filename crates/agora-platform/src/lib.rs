//! The only crate allowed to contain OS-specific code: PTY/ConPTY, process tree, cwd, local sockets and pipes, resource limits.
//!
//! The crate root exposes platform-neutral types and traits; each OS implements them in `src/<os>.rs`.
//! No `#[cfg(target_os)]` or `#[cfg(unix)]` is allowed outside this crate (AGENTS.md).

use std::io;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

mod confined;
#[cfg(windows)]
mod conpty;
pub use confined::{not_confined, valid_component, ConfinedDir, Entry, EntryKind, NotConfined};
mod held;
#[cfg(not(unix))]
mod nohold;
#[cfg(unix)]
mod unix;
#[cfg(unix)]
mod unixhold;
#[cfg(windows)]
mod winconfine;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod winhold;
#[cfg(windows)]
mod winproc;
#[cfg(windows)]
mod winservice;
#[cfg(windows)]
use winproc as procinfo;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macservice;
#[cfg(target_os = "linux")]
use linux as procinfo;
#[cfg(target_os = "macos")]
use macos as procinfo;

/// True where [`local_listen`] and [`local_connect`] work (Unix domain sockets; Windows named pipes, ADR-0045). Callers and tests
/// use this instead of any `cfg`.
pub const LOCAL_SOCKETS_SUPPORTED: bool = cfg!(any(unix, windows));

/// True where [`spawn`] can start a PTY.
pub const PTY_SUPPORTED: bool = cfg!(any(unix, windows));

/// True where [`confine_command`] can confine a plain process (not a PTY): Linux (Landlock) and macOS (Seatbelt), when
/// [`sandbox_supported`] also holds. Windows confines PTY processes only (its restricted token is given at process creation, which
/// `std::process::Command` cannot do), so adapter and ACP agents are refused there before they start (#897).
pub const COMMAND_CONFINEMENT_SUPPORTED: bool = cfg!(any(target_os = "linux", target_os = "macos"));

/// The user's shell, for the node's built-in `shell` kind: on Unix `$SHELL` when it names an existing file, else `sh`; on Windows
/// `%ComSpec%` when set, else `cmd.exe`.
pub fn default_shell() -> String {
    #[cfg(windows)]
    {
        std::env::var("ComSpec")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "cmd.exe".to_string())
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL")
            .ok()
            .filter(|s| std::path::Path::new(s).is_file())
            .unwrap_or_else(|| "sh".to_string())
    }
}

pub use held::{
    Collected, HeldBuffer, HeldControl, HeldLines, HeldSignal, LineChunk, LineWriter, PipeLines,
    QueueError, HELD_BUFFER, HELD_LINE_MAX, LINE_COST, LINE_QUEUE,
};
#[cfg(not(unix))]
pub use nohold::{
    held_pty, HeldProcess, HeldTerminal, ProcessWatch, TerminalHandle, TerminalReader,
};
#[cfg(unix)]
pub use unixhold::{
    held_pty, HeldProcess, HeldTerminal, ProcessWatch, TerminalHandle, TerminalReader,
};

/// ADR-0079 decision 2: makes this process the leader of a session of its own, so nothing that ends its starter's session or group
/// ends it. A process that already leads one is left as it is.
pub fn start_new_session() -> io::Result<()> {
    #[cfg(unix)]
    {
        unixhold::start_new_session()
    }
    #[cfg(not(unix))]
    {
        nohold::start_new_session()
    }
}

/// ADR-0079: closes every descriptor this process was handed at its start but 0, 1 and 2, so nothing of its starter reaches what it
/// starts. Call it first thing in a process that starts agents on someone's behalf. Does nothing on Windows (handles are not
/// inherited unless named).
pub fn close_inherited_descriptors() {
    #[cfg(unix)]
    unixhold::close_inherited_descriptors();
    #[cfg(not(unix))]
    nohold::close_inherited_descriptors();
}

/// The process at the other end of a local connection and whether it runs as this user, as the system recorded them when it
/// connected (ADR-0079 decision 5: the holder compares the pid and start time with the role lock's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peer {
    pub pid: u32,
    pub same_user: bool,
}

/// A connected local (same-machine) stream, used for the node's control socket.
pub struct LocalStream {
    #[cfg(unix)]
    inner: std::os::unix::net::UnixStream,
    #[cfg(windows)]
    inner: windows::Stream,
}

/// What [`LocalStream::set_read_timeout_or_closed`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadTimeout {
    /// The read timeout is set.
    Set,
    /// The system refused a new timeout because the connection is shut (macOS answers `EINVAL` once the peer has closed it), and the
    /// stream is seen to be hung up. A receive on it returns what is buffered or the end at once, so it cannot wait; the timeout is
    /// not set.
    PeerClosed,
}

/// Tells the peer-closed answer of a `set_read_timeout` apart from a real failure: an `InvalidInput` answer is the peer having
/// closed only when `hung_up` confirms the connection is shut; otherwise (or when the check fails) it is returned as it came.
fn read_timeout_result(
    r: io::Result<()>,
    hung_up: impl FnOnce() -> io::Result<bool>,
) -> io::Result<ReadTimeout> {
    match r {
        Ok(()) => Ok(ReadTimeout::Set),
        Err(e) if e.kind() == io::ErrorKind::InvalidInput => match hung_up() {
            Ok(true) => Ok(ReadTimeout::PeerClosed),
            Ok(false) | Err(_) => Err(e),
        },
        Err(e) => Err(e),
    }
}

/// A listening local socket. Removes its socket file when dropped.
pub struct LocalListener {
    #[cfg(unix)]
    inner: std::os::unix::net::UnixListener,
    #[cfg(unix)]
    path: PathBuf,
    /// `<path>.lock`, held while this listener serves the socket (see `unix::local_listen`).
    #[cfg(unix)]
    _lock: std::fs::File,
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

/// Renames `from` to `to` only when nothing has the name `to` (a file, a folder, or a link, which is never followed): never replaces.
/// `AlreadyExists` when the name is taken; `Unsupported` where the system or the file system cannot (the caller falls back). Linux
/// `renameat2(RENAME_NOREPLACE)`, macOS `renamex_np(RENAME_EXCL)`, Windows `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`. For a
/// file written whole beside its place and moved there in one step, never with two names (#1150).
pub fn rename_no_replace(from: &std::path::Path, to: &std::path::Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::rename_no_replace(from, to)
    }
    #[cfg(target_os = "macos")]
    {
        macos::rename_no_replace(from, to)
    }
    #[cfg(windows)]
    {
        windows::rename_no_replace(from, to)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (from, to);
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// The bytes free for this user on the file system that holds `path` (ADR-0064: the node refuses to make a worktree when too little is
/// left). Unix `statvfs` (`f_bavail`, the blocks an unprivileged user may still use, times `f_frsize`), Windows `GetDiskFreeSpaceExW`
/// (the caller's quota included). `path` must exist.
pub fn free_space(path: &std::path::Path) -> io::Result<u64> {
    #[cfg(unix)]
    {
        unix::free_space(path)
    }
    #[cfg(windows)]
    {
        windows::free_space(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// Puts a freshly downloaded program in place of the one at `target` (which may be running): the new file is made executable and
/// renamed over it. Windows cannot replace a running program, so the old one is moved aside to `<target>.old` first; when that name is
/// still taken by a program an earlier update moved aside and that still runs (a node not restarted since), to `<target>.old-<pid>`.
pub fn replace_executable(new: &std::path::Path, target: &std::path::Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(new, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(new, target)
    }
    #[cfg(not(unix))]
    {
        let aside = |suffix: &str| {
            let mut old = target.as_os_str().to_owned();
            old.push(suffix);
            std::path::PathBuf::from(old)
        };
        let mut old = aside(".old");
        if std::fs::remove_file(&old).is_err() && old.exists() {
            // The program an earlier update moved aside still runs (or a virus scanner still holds a file it just saw): take the first
            // free name instead, never one that is still held (renaming over a held file fails with a sharing violation).
            let pid = std::process::id();
            old = (0..100)
                .map(|n| aside(&format!(".old-{pid}-{n}")))
                .find(|p| std::fs::remove_file(p).is_ok() || !p.exists())
                .ok_or_else(|| io::Error::from(io::ErrorKind::AlreadyExists))?;
        }
        // A file just written or renamed is often held for a moment by a virus scanner or the search indexer (a sharing violation,
        // error 32): wait for it briefly instead of failing the update.
        let rename = |from: &std::path::Path, to: &std::path::Path| {
            let mut tries = 0;
            loop {
                match std::fs::rename(from, to) {
                    Err(e) if e.raw_os_error() == Some(32) && tries < 40 => {
                        tries += 1;
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    r => return r,
                }
            }
        };
        rename(target, &old)?;
        rename(new, target).inspect_err(|_| {
            let _ = rename(&old, target);
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

/// True where a shell script with a `#!` line can be the program an agent entry names (tests of the agent installer use a script as
/// a fake agent and skip themselves elsewhere).
pub const SHELL_SCRIPTS_RUN: bool = cfg!(unix);

/// True where a file lock (`File::lock`, `File::try_lock`) keeps every other handle from reading or writing the locked file and a file
/// open through [`open_lock_private`] can be neither renamed nor removed (Windows: `LockFileEx` is mandatory and the file is not
/// shared for delete). Elsewhere a lock is advisory: the file can be read, renamed and removed while it is held.
pub const LOCK_FILES_MANDATORY: bool = cfg!(windows);

/// What a program's file name ends with here: `.exe` on Windows, nothing elsewhere.
pub const EXECUTABLE_SUFFIX: &str = if cfg!(windows) { ".exe" } else { "" };

/// The device name to pass to programs that take an output path when the output should be discarded.
pub const NULL_DEVICE: &str = {
    #[cfg(unix)]
    {
        unix::NULL_DEVICE
    }
    #[cfg(windows)]
    {
        windows::NULL_DEVICE
    }
};

/// The platform key of the signed agent list (`<os>-<arch>[-musl]`: `linux-x86_64`, `darwin-aarch64`, `windows-x86_64`,
/// `linux-aarch64-musl`), or `None` for a system the list has no keys for.
pub fn platform_key() -> Option<String> {
    let os = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(windows) {
        "windows"
    } else {
        return None;
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        return None;
    };
    let musl = if cfg!(target_env = "musl") {
        "-musl"
    } else {
        ""
    };
    Some(format!("{os}-{arch}{musl}"))
}

/// The number of the next temporary name this process takes beside a file it replaces.
static NEXT_TMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A temporary name no other write of this process uses at the same time. A process killed half-way leaves its name behind,
/// and a later process may get the same pid (a container's first process always does), so the caller clears that name first.
fn unique_tmp_suffix() -> String {
    let seq = NEXT_TMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!(".tmp-{}-{seq}", std::process::id())
}

/// The file a temporary name of [`write_private`] stands for and the pid of the process that made it: `<base>.tmp-<pid>-<seq>`, or
/// `<base>.tmp-<pid>` as earlier versions named it (digits only). Anything else is `None`, so a sweeper never takes a stranger's file.
pub fn split_temporary(name: &str) -> Option<(&str, u32)> {
    let (base, tail) = name.rsplit_once(".tmp-")?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let pid = match tail.split_once('-') {
        Some((pid, seq)) if digits(pid) && digits(seq) => pid,
        None if digits(tail) => tail,
        _ => return None,
    };
    Some((base, pid.parse().ok()?))
}

/// Puts a launcher for the program `real` at `link`, replacing what is there in one step, never by editing in place: a symbolic
/// link on Unix (pointing at `relative`, the path from the link's folder, so the folder can move), a copy of the program on Windows
/// (a symbolic link needs a privilege there). The new launcher is made beside `link` under a temporary name and renamed over it.
pub fn install_launcher(
    real: &std::path::Path,
    relative: &std::path::Path,
    link: &std::path::Path,
) -> io::Result<()> {
    let mut tmp = link.as_os_str().to_owned();
    tmp.push(unique_tmp_suffix());
    let tmp = PathBuf::from(tmp);
    let _ = std::fs::remove_file(&tmp);
    let made = {
        #[cfg(unix)]
        {
            let _ = real;
            std::os::unix::fs::symlink(relative, &tmp)
        }
        #[cfg(not(unix))]
        {
            let _ = relative;
            std::fs::copy(real, &tmp).map(|_| ())
        }
    };
    made.and_then(|()| std::fs::rename(&tmp, link))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
}

/// A `PATH` for a program started with an empty environment: the system's own program folders only (nothing of the user's).
pub fn safe_search_path() -> String {
    if cfg!(windows) {
        std::env::var("SystemRoot").map_or_else(|_| String::new(), |r| format!("{r}\\System32;{r}"))
    } else {
        "/usr/bin:/bin".to_string()
    }
}

/// Whether a process with this id exists (a lock file's owner). Only "no such process" (on Windows also: a process that has ended,
/// even while some handle keeps it queryable) says no; any other answer (access denied, a system that cannot tell) says yes, so a lock
/// is never broken on a guess.
pub fn process_exists(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(p) = i32::try_from(pid) else {
            return true;
        };
        if p <= 0 {
            return true;
        }
        // SAFETY: signal 0 only checks that the process exists and may be signalled; nothing is sent.
        if unsafe { libc::kill(p, 0) } == 0 {
            return true;
        }
        // ESRCH is the one answer that means "no such process" (EPERM: it exists, owned by someone else). This is the same on every
        // Unix, unlike the name lookups, whose error for a dead pid differs (macOS leaves a stale errno).
        io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
    #[cfg(windows)]
    {
        winproc::exists(pid)
    }
    #[cfg(not(any(unix, windows)))]
    {
        match process_name(pid) {
            Ok(_) => true,
            Err(e) => e.kind() != io::ErrorKind::NotFound,
        }
    }
}

/// Opens `path` for reading without waiting on it: on Unix with `O_NONBLOCK` (a FIFO or a device swapped in for a file would
/// otherwise block the open until a writer appears; regular files read as usual) and `O_NOCTTY`. Check what was opened on the handle
/// (`metadata().is_file()`, [`opened_path`]).
pub fn open_read(path: &std::path::Path) -> io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::File::open(path)
    }
}

/// [`open_read`], but a link at the last component is opened as itself, never followed (Unix `O_NOFOLLOW`, which refuses it; Windows
/// `FILE_FLAG_OPEN_REPARSE_POINT`, whose handle is then not a regular file). For a file that must be the one a check just looked at:
/// check `metadata().is_file()` on the handle.
pub fn open_read_no_follow(path: &std::path::Path) -> io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_NOFOLLOW)
            .open(path)
    }
    #[cfg(windows)]
    {
        windows::open_read_no_follow(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        std::fs::File::open(path)
    }
}

/// The contents of the regular file at `path`, at most `max` bytes, opened once with [`open_read_no_follow`]: a link at the last
/// component, a FIFO, a device or a folder there is refused (checked on the open handle, so a swap after a look at the path does not
/// matter), never waited on, and a file longer than `max` (a sparse file costs no disk) is refused without being read past `max`.
/// For small files another user or an agent may write, such as git's link files.
pub fn read_small_regular_file_no_follow(path: &std::path::Path, max: u64) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let file = open_read_no_follow(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    if meta.len() > max {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "file too long"));
    }
    let mut out = Vec::new();
    // Grown since the look at its length: one byte past `max` says so.
    file.take(max.saturating_add(1)).read_to_end(&mut out)?;
    if out.len() as u64 > max {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "file too long"));
    }
    Ok(out)
}

/// Whether a file could have been changed by someone other than this user (ADR-0087 item 3): on Unix it belongs to another user or is
/// writable by its group or by everyone. Elsewhere `false`: the Windows install folder is protected by its owner-only ACL.
pub fn others_can_write(meta: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // SAFETY: plain call, no arguments.
        let me = unsafe { libc::geteuid() };
        meta.uid() != me || meta.permissions().mode() & 0o022 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        false
    }
}

/// Whether a file's owner may run it, as git records it (`100755`): the owner's execute bit on Unix; elsewhere never.
pub fn is_executable(meta: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o100 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        false
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

/// Takes every other user's access away from `path` without giving its owner anything new (issue #877: node files an older version
/// left open): Unix clears the group and other mode bits, so a folder kept `0555` stays read-only and a `0000` one stays shut;
/// Windows sets the owner-only ACL of [`restrict_to_owner`] (passed on to what inherits, for a folder). `InvalidInput` for a link, so
/// a link planted at one of the node's paths never closes some other folder, and for a Unix folder others may write in (sticky or
/// other-writable, such as a shared socket folder): it is shared on purpose and is left as it is. A path another user owns fails as
/// the system says. The link check and the change are two steps: whoever could swap the path between them can already write in the
/// node's home. Not covered: macOS extended ACL entries (`chmod +a`), which mode bits do not remove.
pub fn close_to_others(path: &std::path::Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    #[cfg(unix)]
    {
        unix::close_to_others(path, &meta)
    }
    #[cfg(windows)]
    {
        windows::set_owner_only(path, meta.is_dir())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = meta;
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
        tmp.push(unique_tmp_suffix());
        let tmp = std::path::PathBuf::from(tmp);
        let _ = std::fs::remove_file(&tmp);
        let result = (|| {
            #[cfg(unix)]
            let mut f = {
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
                options.open(&tmp)?
            };
            // Private from its creation on, not made private just after it (#320), and not shared while it is written.
            #[cfg(windows)]
            let mut f = windows::create_private_new(&tmp, 0)?;
            f.write_all(contents)?;
            f.sync_all()?;
            // Windows cannot rename a file that is still open.
            drop(f);
            std::fs::rename(&tmp, path)?;
            sync_parent_dir(path);
            Ok(())
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

/// Makes a rename or a new name in `path`'s folder last through a power loss or a hard reset: Unix syncs the folder itself (a file's
/// own `sync_all` does not cover the name that points at it). Windows records the name with the rename, and there this does nothing.
/// Best effort: a folder that cannot be opened or synced does not undo what was done in it. For a file replaced whole by hand (written
/// to a temporary, synced, renamed over it); [`write_private`] already does this.
pub fn sync_parent_dir(path: &std::path::Path) {
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        let dir = if dir.as_os_str().is_empty() {
            std::path::Path::new(".")
        } else {
            dir
        };
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Moves a saved file that could not be read out of the way, keeping it for a person to look at, so the program starts with a fresh one
/// instead of losing it or refusing to start: `path` becomes `<path>.<tag>` in the same folder (`-2`, `-3`, ... added when that name is
/// taken; an existing file is never replaced), and the rename is synced ([`sync_parent_dir`]). `tag` is reduced to `[A-Za-z0-9._-]`
/// (others become `_`, at most 64 characters). Returns the new path. `NotFound` when there is no file at `path`.
pub fn set_aside(path: &std::path::Path, tag: &str) -> io::Result<std::path::PathBuf> {
    let tag: String = tag
        .chars()
        .take(64)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let tag = if tag.is_empty() { "_".to_string() } else { tag };
    std::fs::symlink_metadata(path)?;
    let named = |n: u32| {
        let mut name = path.as_os_str().to_owned();
        name.push(".");
        name.push(&tag);
        if n > 1 {
            name.push(format!("-{n}"));
        }
        std::path::PathBuf::from(name)
    };
    let mut n = 1;
    let mut aside = named(n);
    // The names a person may have kept from earlier times stay theirs. Checked, then renamed: the folder is the program's own.
    while std::fs::symlink_metadata(&aside).is_ok() {
        n += 1;
        if n > 1000 {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        aside = named(n);
    }
    std::fs::rename(path, &aside)?;
    sync_parent_dir(&aside);
    Ok(aside)
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

/// Whether `e` says a path never resolves because of its symbolic links: a link to itself or a ring of links ("too many levels of
/// symbolic links"). Rust has no stable `ErrorKind` for it, so the system's own code is compared. False on systems without links.
pub fn link_loop(e: &io::Error) -> bool {
    #[cfg(unix)]
    {
        unix::link_loop(e)
    }
    #[cfg(windows)]
    {
        windows::link_loop(e)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = e;
        false
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

/// Whether `m` (from `symlink_metadata`) is a reparse point: a Windows junction or symbolic link. Always false on other systems, where
/// [`std::fs::FileType::is_symlink`] says it.
pub fn is_reparse_point(m: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        std::os::windows::fs::MetadataExt::file_attributes(m) & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        let _ = m;
        false
    }
}

/// Creates a link at `link` to the folder `target` that any user may make: a symbolic link on Unix, a junction on Windows (where a
/// symbolic link needs a privilege). Tests that swap a folder for a link use it, so they run on every system instead of skipping.
pub fn link_dir(target: &std::path::Path, link: &std::path::Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        windows::junction(target, link)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        Err(unsupported_io("folder links"))
    }
}

/// The path of the file `file` has open, as the kernel knows it now, with every link resolved (spelled as `std::fs::canonicalize`
/// spells it). Checking a path and then opening it are two steps another process can change in between (a folder or the file
/// itself swapped for a link); checking where the open handle really is happens after the open, so nothing can change it.
pub fn opened_path(file: &std::fs::File) -> io::Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        macos::opened_path(file)
    }
    #[cfg(target_os = "linux")]
    {
        linux::opened_path(file)
    }
    #[cfg(windows)]
    {
        windows::opened_path(file)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = file;
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// One entry of a folder listed with [`list_dir`], described without following links (as `DirEntry::file_type` and
/// `DirEntry::metadata` describe it): a link is neither a folder nor its target's size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirItem {
    pub name: std::ffi::OsString,
    pub is_dir: bool,
    pub len: u64,
    pub modified: std::time::SystemTime,
}

/// Opens the folder at `path` for [`list_dir`] (following links, as any open does: check where it is with [`opened_path`]).
pub fn open_dir(path: &std::path::Path) -> io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        unix::open_dir(path)
    }
    #[cfg(windows)]
    {
        windows::open_dir(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// A folder held open from the moment a start checked it (ADR-0052): [`spawn_in`] starts the process in exactly this folder, never in
/// what its path names by then. Between the check and the start, whoever can write beside the folder (another agent of the project)
/// could swap it, or a folder above it, for a link to another project's folder, and a start by path would follow that link (F1012).
#[derive(Debug)]
pub struct HeldFolder {
    dir: std::fs::File,
    path: PathBuf,
}

/// The error inside an [`io::Error`] from [`HeldFolder::open`] or [`spawn_in`]: the path no longer names the held folder (it was moved,
/// replaced by a link, or a folder above it was), so nothing was started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderMoved;

impl std::fmt::Display for FolderMoved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("folder_moved")
    }
}

impl std::error::Error for FolderMoved {}

/// The error inside an [`io::Error`] from [`spawn`] on Windows: an argument to a batch file (`.bat`, `.cmd`) holds a line break or a NUL,
/// which `cmd.exe` cannot carry without ending the command, so nothing was started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchArgumentRefused;

impl std::fmt::Display for BatchArgumentRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("batch_argument_refused")
    }
}

impl std::error::Error for BatchArgumentRefused {}

/// Whether `e` (from [`spawn`]) says an argument to a batch file could not be carried to it.
pub fn batch_argument_refused(e: &io::Error) -> bool {
    e.get_ref()
        .is_some_and(|inner| inner.is::<BatchArgumentRefused>())
}

/// Whether `e` (from [`HeldFolder::open`] or [`spawn_in`]) says the folder was no longer at its path.
pub fn folder_moved(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|inner| inner.is::<FolderMoved>())
}

impl HeldFolder {
    /// Opens the folder at `path`, an absolute path spelled as [`canonicalize_plain`] spells it (the spelling a check records), and
    /// keeps it only when the opened folder is at exactly that path: a link swapped in anywhere along it leads elsewhere, so it is
    /// refused ([`folder_moved`]), never followed. On Windows the folder is held without delete sharing, so it can be neither renamed
    /// nor removed while it is held.
    pub fn open(path: &std::path::Path) -> io::Result<Self> {
        // Linux: held as a place only, so a folder the user may enter but not list can be held (and started in) too.
        #[cfg(target_os = "linux")]
        let dir = linux::open_held_dir(path)?;
        #[cfg(all(unix, not(target_os = "linux")))]
        let dir = unix::open_dir(path)?;
        #[cfg(windows)]
        let dir = windows::open_dir_held(path)?;
        let held = Self {
            dir,
            path: path.to_path_buf(),
        };
        if !held.in_place() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, FolderMoved));
        }
        Ok(held)
    }

    /// The path the folder was held at.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Whether the path still names the held folder: it was not moved or removed, and neither it nor a folder above it was swapped for
    /// a link (where the open handle really is, as the kernel knows it now, is compared with the path).
    pub fn in_place(&self) -> bool {
        let Ok(now) = opened_path(&self.dir) else {
            return false;
        };
        #[cfg(windows)]
        let now = windows::plain_path(now);
        now == self.path
    }
}

/// The entries of the folder `dir` holds open (from [`open_dir`]), read through that handle: `std::fs::read_dir` takes a path and
/// opens it again, so a folder swapped for a link after its handle was checked would be listed instead. `.` and `..` are left out.
pub fn list_dir(dir: &std::fs::File) -> io::Result<Vec<DirItem>> {
    let mut items = Vec::new();
    visit_dir(dir, |item| {
        items.push(item);
        std::ops::ControlFlow::Continue(())
    })?;
    Ok(items)
}

/// Visits entries through the open directory handle. Returning [`std::ops::ControlFlow::Break`] stops enumeration before
/// reading metadata for any later entry. Unlike [`list_dir`], this never collects the entire directory first.
pub fn visit_dir(
    dir: &std::fs::File,
    mut visitor: impl FnMut(DirItem) -> std::ops::ControlFlow<()>,
) -> io::Result<()> {
    #[cfg(unix)]
    {
        unix::visit_dir(dir, &mut visitor)
    }
    #[cfg(windows)]
    {
        windows::visit_dir(dir, &mut visitor)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (dir, visitor);
        Err(io::Error::from(io::ErrorKind::Unsupported))
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
    /// Wait for the next client. The stream it returns blocks (its reads wait, up to their timeout), whether or not this listener
    /// does.
    pub fn accept(&self) -> io::Result<LocalStream> {
        #[cfg(unix)]
        {
            // macOS and the BSDs hand an accepted socket the listener's O_NONBLOCK (Linux does not). A listener made non-blocking
            // for its accept loop must not make every connection non-blocking: a read with a timeout would return WouldBlock at
            // once, and a loop that waits on it spins a core (seen on macOS: each idle control connection at 100% CPU).
            let (inner, _) = self.inner.accept()?;
            inner.set_nonblocking(false)?;
            Ok(LocalStream { inner })
        }
        #[cfg(windows)]
        {
            self.inner.accept().map(|inner| LocalStream { inner })
        }
    }

    /// Wait up to `timeout` for the next client, and return it as soon as it connects (a readiness wait, not a sleep between tries);
    /// `None` when none came in that time. For an accept loop that must also notice a stop: the client is served at once and the
    /// stop is seen within `timeout`. Make the listener non-blocking first ([`set_nonblocking`](Self::set_nonblocking)) so a client
    /// that leaves between the wait and the accept cannot leave this waiting past `timeout` on Unix.
    pub fn accept_timeout(&self, timeout: std::time::Duration) -> io::Result<Option<LocalStream>> {
        #[cfg(unix)]
        {
            if !unix::wait_readable(&self.inner, timeout)? {
                return Ok(None);
            }
            match self.accept() {
                Ok(s) => Ok(Some(s)),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
                Err(e) => Err(e),
            }
        }
        #[cfg(windows)]
        {
            self.inner
                .accept_timeout(timeout)
                .map(|s| s.map(|inner| LocalStream { inner }))
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

    /// [`Self::set_read_timeout`] for a caller that can go on without the timeout once the peer has shut the connection: that case
    /// (an `InvalidInput` answer, macOS, on a stream that is seen to be hung up: `poll(2)` on Unix) is `Ok(ReadTimeout::PeerClosed)`;
    /// every other failure is returned, an `InvalidInput` on a stream still open too. Windows never refuses a read timeout.
    pub fn set_read_timeout_or_closed(
        &self,
        timeout: Option<std::time::Duration>,
    ) -> io::Result<ReadTimeout> {
        // std refuses a zero timeout with InvalidInput; that is an error of the caller, not the peer having closed.
        if timeout.is_some_and(|t| t.is_zero()) {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        read_timeout_result(self.set_read_timeout(timeout), || self.hung_up())
    }

    /// Whether the connection is shut in both directions (see [`Self::set_read_timeout_or_closed`]).
    fn hung_up(&self) -> io::Result<bool> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            unixhold::hung_up(self.inner.as_raw_fd())
        }
        #[cfg(windows)]
        {
            Ok(false)
        }
    }

    /// Waits until a send would not block, at most `timeout`; `Ok(false)` when the time ran out first. Unix: `poll(2)` for
    /// writability (an error or hang-up counts as ready, so the send itself reports it). Windows: pipe writes are not polled, so
    /// this returns `Ok(true)` at once and the send is bounded by its write timeout ([`Self::set_write_timeout`]).
    pub fn wait_writable(&self, timeout: std::time::Duration) -> io::Result<bool> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            unixhold::wait_writable(self.inner.as_raw_fd(), timeout)
        }
        #[cfg(windows)]
        {
            let _ = timeout;
            Ok(true)
        }
    }

    /// How long a send may wait for the peer to take what it sent (`None`: for ever). Past it the send fails with `WouldBlock` or
    /// `TimedOut`, having sent nothing, so it can be tried again. A zero timeout is refused (`InvalidInput`). Windows: the pending
    /// pipe write is cancelled at the timeout, and while a timeout is set one send writes at most 16 KiB, because only a write no
    /// larger than the pipe's buffer is sure to have sent nothing when it is cancelled (measured, #1089).
    pub fn set_write_timeout(&self, timeout: Option<std::time::Duration>) -> io::Result<()> {
        #[cfg(unix)]
        {
            self.inner.set_write_timeout(timeout)
        }
        #[cfg(windows)]
        {
            self.inner.set_write_timeout(timeout)
        }
    }

    /// The process at the other end (on a listener's accepted stream: the client that connected).
    pub fn peer(&self) -> io::Result<Peer> {
        #[cfg(unix)]
        {
            unixhold::peer_of(std::os::fd::AsRawFd::as_raw_fd(&self.inner))
        }
        #[cfg(windows)]
        {
            // A pipe made owner-only admits only this user.
            Ok(Peer {
                pid: self.inner.client_pid()?,
                same_user: true,
            })
        }
    }

    /// One send of `bytes` with `terminal` (a master from [`HeldTerminal::handle`]) attached when given; returns how many bytes went.
    /// The terminal goes with the first byte that does: send the rest with `write`. Unix `SCM_RIGHTS`.
    pub fn send_with_terminal(
        &mut self,
        bytes: &[u8],
        terminal: Option<&TerminalHandle>,
    ) -> io::Result<usize> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            unixhold::send_with_fd(
                self.inner.as_raw_fd(),
                bytes,
                terminal.map(|t| t.fd.as_raw_fd()),
            )
        }
        #[cfg(not(unix))]
        {
            let _ = (bytes, terminal);
            Err(unsupported_io("passing a terminal"))
        }
    }

    /// One receive into `buf` that also takes a terminal sent with the bytes. Read a message in exact sizes, so a terminal is never
    /// taken for another message.
    pub fn recv_with_terminal(
        &mut self,
        buf: &mut [u8],
    ) -> io::Result<(usize, Option<TerminalHandle>)> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let (n, fd) = unixhold::recv_with_fd(self.inner.as_raw_fd(), buf)?;
            Ok((n, fd.map(|fd| TerminalHandle { fd })))
        }
        #[cfg(not(unix))]
        {
            let _ = buf;
            Err(unsupported_io("receiving a terminal"))
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

/// For a full-screen program that has put the terminal into `raw` mode: from now on `SIGTERM`, `SIGINT` and `SIGHUP` first write
/// `reset` to standard output and put the terminal's settings back, and then end the process by that signal, as they did before (so
/// no loop of the program has to poll for them). On Windows there is no such signal for a console program and this does nothing.
pub fn restore_terminal_on_stop(raw: &RawMode, reset: &'static [u8]) -> io::Result<()> {
    #[cfg(unix)]
    {
        unix::restore_terminal_on_stop(&raw.saved, reset)
    }
    #[cfg(not(unix))]
    {
        let _ = (raw, reset);
        Ok(())
    }
}

/// Reads one line from the terminal on stdin without showing what is typed (a password), without its line ending. Fails with
/// `Unsupported` when stdin is not a terminal (a secret is never read from a pipe or a file by accident) and with `Interrupted` on
/// Ctrl-C; the terminal's settings are restored on every path, Ctrl-C included. A caller that shows a prompt should use
/// [`read_secret_line_with_prompt`]: a prompt printed before this call can be answered before echo is off, and that answer is shown.
pub fn read_secret_line() -> io::Result<String> {
    read_secret(None)
}

/// [`read_secret_line`] that writes `prompt` (and a line break) to stderr only once echo is off, so nothing typed or pasted in answer
/// to it is ever shown. On Unix, input that arrived before the prompt (typed ahead, and already shown by the terminal as it came) is
/// discarded when echo is switched off, so it never becomes part of the secret; the person types it again after the prompt.
pub fn read_secret_line_with_prompt(prompt: &str) -> io::Result<String> {
    read_secret(Some(prompt))
}

fn read_secret(prompt: Option<&str>) -> io::Result<String> {
    #[cfg(unix)]
    let mode = unix::secret_mode(prompt.is_some())?;
    #[cfg(windows)]
    let mode = windows::secret_mode()?;
    #[cfg(not(any(unix, windows)))]
    let mode: () = return Err(unsupported_io("reading a password"));
    if let Some(prompt) = prompt {
        use std::io::Write;
        let mut err = io::stderr().lock();
        // A prompt that cannot be shown does not stop the read: the terminal is still the one typed into.
        let _ = writeln!(err, "{prompt}");
        let _ = err.flush();
    }
    let line = secret_from(&mut io::stdin().lock());
    drop(mode);
    // The Enter (or Ctrl-C) that ended it was not echoed.
    eprintln!();
    line
}

/// The line editing [`read_secret_line`] does itself, with the terminal's own switched off: Enter ends the line, Backspace removes the
/// last character, Ctrl-U the whole line, Ctrl-C is `Interrupted`, Ctrl-D on an empty line is `UnexpectedEof`; other control bytes
/// are dropped.
pub fn secret_from(input: &mut impl io::Read) -> io::Result<String> {
    let mut line: Vec<u8> = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if input.read(&mut byte)? == 0 {
            if line.is_empty() {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            break;
        }
        match byte[0] {
            b'\r' | b'\n' => break,
            0x03 => return Err(io::ErrorKind::Interrupted.into()),
            0x04 if line.is_empty() => return Err(io::ErrorKind::UnexpectedEof.into()),
            0x15 => line.clear(),
            0x08 | 0x7f => {
                // One character, however many bytes it took.
                while line.pop().is_some_and(|b| (0x80..0xc0).contains(&b)) {}
            }
            b if b < 0x20 => {}
            b => line.push(b),
        }
    }
    String::from_utf8(line).map_err(|_| io::ErrorKind::InvalidData.into())
}

/// Reads one answer line from the **terminal** on stdin, for a decision that must come from a person at the keyboard (ADR-0065: the
/// owner comparing two numbers). It refuses what a program can feed:
/// - stdin that is not a terminal (a pipe, a file) is `Unsupported`, never read;
/// - the input typed, held or sent before now is discarded when the call starts (`tcflush` on Unix, `FlushConsoleInputBuffer` on
///   Windows), so what was queued cannot answer;
/// - if the first key arrives sooner than `min_wait` after that, the answer is `Ok(None)`: a held or scripted key, not someone who
///   read the screen. Nothing more is read then;
/// - the line is at most 256 bytes, so a key held without Enter cannot grow it.
///
/// Nothing is echoed; the terminal's settings are restored on every path.
pub fn read_confirmation(min_wait: std::time::Duration) -> io::Result<Option<String>> {
    use std::io::Read;
    #[cfg(unix)]
    let mode = unix::secret_mode(true)?;
    #[cfg(windows)]
    let mode = {
        let m = windows::secret_mode()?;
        windows::flush_input();
        m
    };
    #[cfg(not(any(unix, windows)))]
    let mode: () = return Err(unsupported_io("reading an answer"));
    let start = std::time::Instant::now();
    let mut stdin = io::stdin().lock();
    let line = (|| {
        let mut first = [0u8; 1];
        if stdin.read(&mut first)? == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        if start.elapsed() < min_wait {
            return Ok(None);
        }
        let mut rest = io::Cursor::new(first).chain((&mut stdin).take(255));
        secret_from(&mut rest).map(Some)
    })();
    drop(mode);
    line
}

/// What [`read_confirmation_until`] got.
#[derive(Debug, PartialEq, Eq)]
pub enum Confirmation {
    /// A line, ended with Enter.
    Line(String),
    /// The first key came sooner than the minimum wait: a held or scripted key, not an answer. Nothing more was read.
    TooSoon,
    /// The caller's check said the question is over (it was answered somewhere else) before a line was ended. Nothing typed counts.
    Stopped,
}

/// How long [`read_confirmation_until`] waits for a key before it asks its caller again whether to go on.
const KEY_WAIT: std::time::Duration = std::time::Duration::from_millis(50);

/// [`read_confirmation`] that can be ended from outside (#942): `stop` is called before each wait for a key, at least every
/// [`KEY_WAIT`], and when it returns true the read ends with [`Confirmation::Stopped`], whatever was typed so far. The question
/// itself is unchanged: the terminal only (a pipe or a file is `Unsupported`), what was typed before the call is discarded, a first
/// key sooner than `min_wait` is [`Confirmation::TooSoon`], the line is at most 256 bytes, nothing is echoed and the terminal's
/// settings are restored on every path. The keys are read from the terminal directly (not through the buffer of `std::io::stdin`),
/// so nothing read here is left behind for a later read, and a key that arrives is never hidden from the wait.
pub fn read_confirmation_until(
    min_wait: std::time::Duration,
    stop: &mut dyn FnMut() -> bool,
) -> io::Result<Confirmation> {
    #[cfg(unix)]
    let mode = unix::secret_mode(true)?;
    #[cfg(windows)]
    let mode = {
        let m = windows::secret_mode()?;
        windows::flush_input();
        m
    };
    #[cfg(not(any(unix, windows)))]
    let mode: () = return Err(unsupported_io("reading an answer"));
    let start = std::time::Instant::now();
    #[cfg(unix)]
    let mut key = unix::terminal_key;
    #[cfg(windows)]
    let mut key = windows::terminal_key;
    let got = confirmation_from(&mut key, start, min_wait, stop);
    drop(mode);
    got
}

/// The keys of a terminal as a reader: waits for the next key in steps of [`KEY_WAIT`], asking `stop` before each step.
struct Keys<'a> {
    /// The bytes of the keys that came within the wait (`None` when none came; empty at the end of input).
    key: &'a mut dyn FnMut(std::time::Duration) -> io::Result<Option<Vec<u8>>>,
    stop: &'a mut dyn FnMut() -> bool,
    stopped: bool,
    held: std::collections::VecDeque<u8>,
}

impl io::Read for Keys<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let Some(out) = buf.first_mut() else {
            return Ok(0);
        };
        loop {
            if let Some(b) = self.held.pop_front() {
                *out = b;
                return Ok(1);
            }
            if (self.stop)() {
                self.stopped = true;
                return Err(io::Error::other("the question was answered elsewhere"));
            }
            match (self.key)(KEY_WAIT)? {
                None => {}
                Some(k) if k.is_empty() => return Ok(0),
                Some(k) => self.held.extend(k),
            }
        }
    }
}

/// The rules of [`read_confirmation_until`] over any source of keys, so they are tested without a terminal. `start` is when the
/// question was shown (after the typed-ahead input was discarded).
fn confirmation_from(
    key: &mut dyn FnMut(std::time::Duration) -> io::Result<Option<Vec<u8>>>,
    start: std::time::Instant,
    min_wait: std::time::Duration,
    stop: &mut dyn FnMut() -> bool,
) -> io::Result<Confirmation> {
    use std::io::Read;
    let mut keys = Keys {
        key,
        stop,
        stopped: false,
        held: Default::default(),
    };
    let got = (|| {
        let mut first = [0u8; 1];
        if keys.read(&mut first)? == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        if start.elapsed() < min_wait {
            return Ok(Confirmation::TooSoon);
        }
        let mut rest = io::Cursor::new(first).chain((&mut keys).take(255));
        secret_from(&mut rest).map(Confirmation::Line)
    })();
    match got {
        Err(_) if keys.stopped => Ok(Confirmation::Stopped),
        other => other,
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
    /// Inherited environment variables the process must not get: exact names, or a prefix ending in `*` (`CLAUDE_CODE_*`). Variables
    /// in `env` are set whatever this says.
    pub env_remove: Vec<String>,
}

/// Whether an inherited variable `name` is one [`PtyConfig::env_remove`] leaves out (on Windows names compare without case).
pub fn env_removed(name: &std::ffi::OsStr, patterns: &[String]) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let eq = |a: &str, b: &str| {
        if cfg!(windows) {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    patterns.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => {
            name.len() >= prefix.len()
                && name.is_char_boundary(prefix.len())
                && eq(&name[..prefix.len()], prefix)
        }
        None => eq(name, p),
    })
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
    /// The agent's own unix sockets (its mesh socket; offline, also its private temp folder): reachable even below `deny`, and the
    /// only ones reachable when `offline`.
    pub sockets: Vec<PathBuf>,
    /// ADR-0083: the confined process may connect to unix sockets only below `sockets` (and the system's name resolver where the OS
    /// has one), not to an ssh-agent, a session bus, a docker daemon or a tmux server of the user. Where [`socket_confinement`] says
    /// `Full` (macOS) every other pathname and abstract socket is refused; `AbstractOnly` (Linux with Landlock ABI 6 or newer) refuses
    /// abstract sockets made outside the agent's own domain and leaves pathname sockets as they are; `None` (older kernels, Windows)
    /// enforces nothing, and the caller must not pretend it does.
    pub confine_sockets: bool,
    /// The node's own folders and files (its data, identity, logs, control socket, other agents' mesh sockets): never read or written,
    /// and no unix socket below them connected to, even when they sit below an allowed folder (a node home under `/tmp`), except
    /// `sockets` and the dedicated generated Git file, including paths that do not exist yet when the agent starts.
    /// Enforced on macOS only (Seatbelt can say "except");
    /// Linux and Windows ignore it, so there a node home below an allowed folder stays reachable by file rights: on Linux the node
    /// refuses to start an agent whose grants contain the control token file (ADR-0082) or any of these paths
    /// ([`PROTECTED_PATHS_BY_REFUSAL`], #1114), and the platform checks the grants it opens against them again (`closed`, #1150); Windows keeps the files out by its own ACLs (see `docs/security/threat-model.md`, T16).
    pub deny: Vec<PathBuf>,
    /// Where [`PROTECTED_PATHS_BY_REFUSAL`] holds (Linux): the paths the node checked no `read` or `write` grant reaches before the
    /// start (its control token, its folder of stored secrets, the paths it keeps closed and its data folder). Checked again on each
    /// grant as it is opened for the kernel (#1150): a grant whose opened file or folder is one of them, or a folder above one, refuses
    /// the start ([`SandboxRefusal::ProtectedPathReachable`]), so a grant swapped for a link after the node's check cannot reach them.
    /// Ignored elsewhere.
    pub closed: Vec<PathBuf>,
    /// The folders below `deny` the agent may use after all (the worktree the node made for it). Each is reopened only while it is a
    /// real folder whose own name is not a symbolic link: an agent that swapped its worktree for a link to the node's home would
    /// otherwise have the home opened at its next start. A `read` or `write` entry that resolves below `deny` is never opened.
    /// macOS only, as `deny`.
    pub reopen: Vec<PathBuf>,
    /// The node-generated Git configuration (ADR-0098). This grant reads an exact file, including
    /// under a protected directory. Unlike a worktree exception, it grants no writes or listing.
    pub git_config: Option<PathBuf>,
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

    /// The file that can be named in the sandbox and in the launch environment.
    pub fn git_config_file(&self) -> Option<PathBuf> {
        let path = self.git_config.as_deref()?;
        path.to_str()?;
        if !path.is_absolute() || !std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file())
        {
            return None;
        }
        #[cfg(target_os = "macos")]
        {
            macos::git_config_path(path)
        }
        #[cfg(windows)]
        {
            // Git for Windows ignores verbatim paths in GIT_CONFIG_GLOBAL. If the same
            // file has no ordinary spelling, the generated configuration is optional.
            let plain = canonicalize_plain(path).ok()?;
            if matches!(plain.components().next(), Some(std::path::Component::Prefix(p)) if p.kind().is_verbatim())
            {
                return None;
            }
            Some(plain)
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            Some(path.to_path_buf())
        }
    }
}

/// The settings copied by the node take precedence over ambient Git environment overrides.
pub fn git_config_env(file: &std::path::Path) -> [(String, String); 4] {
    [
        ("GIT_CONFIG_COUNT".into(), "0".into()),
        ("GIT_CONFIG_PARAMETERS".into(), String::new()),
        (
            "GIT_CONFIG_GLOBAL".into(),
            file.to_string_lossy().into_owned(),
        ),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ]
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

/// True where the node's control socket needs a token because the system gives a confined agent no other barrier to it (ADR-0082):
/// Linux, whose Landlock rules do not restrict connecting to a pathname socket. macOS closes the node's home in the Seatbelt profile
/// and Windows refuses the pipe to a restricted token. Callers use this instead of a `cfg`.
pub const CONTROL_TOKEN_REQUIRED: bool = cfg!(target_os = "linux");

/// True where [`Sandbox::deny`] is kept only by the node refusing the start (#1114): Linux, whose Landlock rules can only allow, so a
/// node path below a folder an agent is given (a `--data` under `/tmp`, `/var` or `/opt`) would be open to it. There the node refuses
/// to start a confined agent whose read or write grants reach one of its own paths, and the platform checks each grant again as it
/// opens it for Landlock ([`Sandbox::closed`], #1150). macOS enforces `deny` in the Seatbelt profile. On
/// Windows the check does not run either: there the node's files are out of reach when their ACLs leave the agent's restricted token
/// out (the default folders under the user's profile; a data folder that `Users` may read is not checked, #1114). Callers use this
/// instead of a `cfg`.
pub const PROTECTED_PATHS_BY_REFUSAL: bool = cfg!(target_os = "linux");

/// Whether this build has the terminal holder (ADR-0079): Unix. Where it is `false` nothing starts one, and the node runs its agents
/// itself (Windows joins in a later step).
pub const HOLDER_BUILT: bool = cfg!(unix);

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

/// What [`Sandbox::confine_sockets`] does on this machine (ADR-0083).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketConfinement {
    /// Every unix socket outside the allow-list is refused (macOS, Seatbelt).
    Full,
    /// Only abstract unix sockets made outside the agent's own sandbox are refused (Linux, Landlock ABI 6 or newer): the X11 abstract
    /// socket, `unix:abstract=` D-Bus. Pathname sockets (an ssh-agent, the session bus, docker, tmux) stay reachable.
    AbstractOnly,
    /// Nothing is refused (older kernels, Windows, other systems).
    None,
}

/// What [`Sandbox::confine_sockets`] can do here. `agora doctor` says it; the caller may not claim more.
pub fn socket_confinement() -> SocketConfinement {
    #[cfg(target_os = "macos")]
    {
        // Measured on a real Mac (macOS 27.2.1, 2026-11-17): own socket open, another closed, the resolver open.
        if macos::sandbox_supported() {
            return SocketConfinement::Full;
        }
    }
    #[cfg(target_os = "linux")]
    {
        if linux::scopes_abstract_sockets() {
            return SocketConfinement::AbstractOnly;
        }
    }
    SocketConfinement::None
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
    /// One of the node's own paths it must keep closed (`Sandbox::deny`) is relative or cannot be resolved (macOS; Linux: one of
    /// `Sandbox::closed`, or a folder above it, could not be looked at, #1150).
    ProtectedPathUnresolved,
    /// A file or folder the agent may read or write is, as opened for the kernel, one of the node's own paths (`Sandbox::closed`) or a
    /// folder above one (Linux, #1150: a grant swapped for a link after the node checked it).
    ProtectedPathReachable,
    /// The agent's worktree (`Sandbox::reopen`) is no longer a plain folder: a link, or something else, is in its place (macOS).
    WorktreeNotFolder,
    /// A file or folder the agent may write or read is reached through a symbolic link inside a folder it may write, which it could
    /// have made itself to be given where the link points at its next start (#545, macOS).
    WritableIsLink,
    /// A file the agent may write also has another name elsewhere (a hard link made outside confinement), so writing it would change
    /// that other place too (#571). Windows: a regular file in the agent's worktree, outside `node_modules`, `target` and `.git`, whose
    /// links are made on purpose by package stores and git.
    WritableHasLinks,
}

impl SandboxRefusal {
    pub fn token(self) -> &'static str {
        match self {
            Self::BroadWriteGrant => "broad_write_grant",
            Self::LabelFailed => "label_failed",
            Self::ProtectedPathUnresolved => "protected_path_unresolved",
            Self::ProtectedPathReachable => "protected_path_reachable",
            Self::WorktreeNotFolder => "worktree_not_folder",
            Self::WritableIsLink => "writable_is_link",
            Self::WritableHasLinks => "writable_has_links",
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
/// cannot be enforced, so the caller never runs the command unconfined by accident. Returns the
/// generated Git file if the confinement grants read access to it.
pub fn confine_command(
    cmd: &mut std::process::Command,
    sandbox: &Sandbox,
) -> io::Result<Option<PathBuf>> {
    if !enforceable(sandbox) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this machine cannot confine a process",
        ));
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let mut ruleset = linux::ruleset(sandbox, None, None)?;
        let git_config = ruleset.git_config.take();
        // SAFETY: the closure only applies a ruleset made before the fork (async-signal-safe calls).
        unsafe { cmd.pre_exec(move || ruleset.enforce()) };
        Ok(git_config)
    }
    #[cfg(target_os = "macos")]
    {
        // The command becomes `sandbox-exec -p <profile> -- <program> <args>`. Environment and folder carry over; standard streams do
        // not (std cannot read them back), so callers confine first and set the streams after.
        let (program, args, git_config) = {
            let a: Vec<_> = cmd.get_args().map(ToOwned::to_owned).collect();
            macos::wrap(sandbox, cmd.get_program(), &a)?
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
        Ok(git_config)
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
            env_remove: Vec::new(),
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
    /// ADR-0079: the node stops and leaves a held agent running: its terminal stops being read and `kill` does nothing from now on.
    fn detach(&self) {}
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
    /// ADR-0079: lets go of a held agent without ending it (the node is stopping): the read of its terminal ends and its copy of the
    /// master closes. A terminal the node owns outright ignores this (dropping it kills the agent, as before).
    pub fn detach(&self) {
        self.inner.detach();
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

/// ADR-0094: whether a process's short name (from [`process_name`]) is the program name a manifest lists. Windows file names keep
/// their case but do not tell it apart (`CLAUDE.EXE` is `claude.exe`), so there the names are compared ignoring ASCII case; elsewhere
/// they are compared exactly, as the system does.
pub fn same_program_name(process: &str, listed: &str) -> bool {
    if cfg!(windows) {
        process.eq_ignore_ascii_case(listed)
    } else {
        process == listed
    }
}

/// What tells a running process apart from a later one that got the same pid (ADR-0060): its process group, its start time in a
/// unit of the system's own (compared, never shown), the boot it started in (Linux `boot_id`; empty where the start time already
/// tells boots apart) and its program's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub group: u32,
    pub started: u64,
    pub boot: String,
    pub name: String,
}

/// The [`ProcessIdentity`] of a running process. `Unsupported` on Windows, where a node's agents are in a job object that ends them
/// with the node, so none can outlive it to be found again.
pub fn process_identity(pid: u32) -> io::Result<ProcessIdentity> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        procinfo::identity(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        Err(unsupported())
    }
}

/// Whether the process `id` names still runs: the same pid with the same start time, in the same boot, running the same program (a
/// reused pid is not it, and neither is a record that names nothing: an empty name never matches).
pub fn process_still_running(id: &ProcessIdentity) -> bool {
    id.pid > 1
        && !id.name.is_empty()
        && process_identity(id.pid).is_ok_and(|now| {
            now.started == id.started && now.boot == id.boot && now.name == id.name
        })
}

/// Ends the process `id` names (ADR-0060), and with it its process group when it leads one (an agent's terminal does), only if it
/// still runs with the same start time: a pid that was given to another process is never touched. `Ok(false)` when it was not
/// running. Never this process's own group.
pub fn kill_process_if_same(id: &ProcessIdentity) -> io::Result<bool> {
    if !process_still_running(id) {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        let pid =
            i32::try_from(id.pid).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        // SAFETY: getpgrp has no preconditions.
        let own = unsafe { libc::getpgrp() };
        let group = i32::try_from(id.group).unwrap_or(0);
        // SAFETY: plain signals to a process group, or a process, checked to be the recorded one just above.
        let r = if group == pid && group > 1 && group != own {
            unsafe { libc::killpg(group, libc::SIGKILL) }
        } else {
            unsafe { libc::kill(pid, libc::SIGKILL) }
        };
        if r != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(true)
    }
    #[cfg(not(unix))]
    {
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

/// Makes a helper program the node runs for itself (git, and the like) start with no window of its own. The node has no console
/// (it runs detached, ADR-0062), and on Windows every console program started by a process without one gets a new console window:
/// each `git` behind a diff flashed a window and took the focus. Windows: `CREATE_NO_WINDOW`, so the helper gets a console with no
/// window, which the programs it starts share. Elsewhere nothing changes. Not for agents: they run on a terminal of their own.
pub fn no_console_window(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(cmd, winproc::NO_WINDOW_FLAGS);
    cmd
}

/// The program to give [`std::process::Command::new`] for an agent's `argv[0]`. On Windows, `Command` adds only `.exe` to a bare
/// name, so a tool installed as a `.cmd` shim (`npx`, most npm global commands) would not be found: a bare name with no extension
/// and no `<name>.exe` anywhere becomes the full path of the first `.com`, `.bat` or `.cmd` of that name, looked up where `Command`
/// looks (`child_path`, the child's own `PATH` when it sets one, then this program's folder, the system folders and this process's
/// `PATH`; never a current folder). `Command` then runs a `.bat`/`.cmd` through `cmd.exe` with its own safe quoting. Anything
/// else, and every name elsewhere, comes back unchanged.
pub fn program_path(program: &str, child_path: Option<&std::ffi::OsStr>) -> PathBuf {
    #[cfg(windows)]
    {
        windows::program_path(program, child_path)
    }
    #[cfg(not(windows))]
    {
        let _ = child_path;
        PathBuf::from(program)
    }
}

/// Makes a long-lived child of this process (an adapter agent) start with standard input, output and error and no other descriptor
/// of this process, not even one another thread has open without close-on-exec at that moment. Where a pipe is not made close-on-exec
/// in one step (macOS: std makes it, then marks it), a pipe another thread was making as the child started (a `git` whose output the
/// node reads, a terminal agent's start waiting for its `exec` report) stayed open in the child for its whole life, and whoever waited
/// for that pipe's end waited as long: three nodes in one test process hung on it until the CI step's limit (#1011). Unix: every other
/// descriptor is marked close-on-exec between fork and exec. Elsewhere nothing changes.
pub fn keep_only_standard_streams(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(unix)]
    unix::keep_only_standard_streams(cmd);
    cmd
}

/// Makes `cmd` start as the leader of a process group of its own (Unix), so [`kill_process_tree`] ends it and everything it started.
/// Windows: nothing changes.
pub fn own_process_group(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(cmd, 0);
    cmd
}

/// Ends a child started with [`own_process_group`] and, on Unix, every process left in its group (a helper program's own
/// background children), so none of them keeps a pipe of this process open. Also reaps the child.
pub fn kill_process_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: a signal to the process group that `own_process_group` made for this child, whose id is `pid`; a group that is
        // already gone answers an error that is ignored.
        unsafe { libc::killpg(pid, libc::SIGKILL) };
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Starts `cmd` detached from this process, its terminal and its session (ADR-0062): it survives the terminal closing, the SSH session
/// ending and this process being killed. Standard input is empty; standard output and error are appended to `log` (created readable by
/// the user only). Unix: a new session (`setsid`), so no terminal hangup or Ctrl-C reaches it; Windows: `DETACHED_PROCESS` and
/// `CREATE_NEW_PROCESS_GROUP`. Returns its process id. A thread of this process reaps it if it ends while this process still runs,
/// so it never lingers as a zombie. Nothing else of this process is inherited: no other descriptor or handle (a caller reading this
/// process's output to its end is never kept waiting by the daemon).
///
/// On Windows the process also leaves this process's job (`CREATE_BREAKAWAY_FROM_JOB`), so closing the terminal that started it does
/// not end it; when the job forbids that, it is started inside the job and [`Detached::in_job`] says so.
pub fn spawn_detached(
    cmd: &mut std::process::Command,
    log: &std::path::Path,
) -> io::Result<Detached> {
    // The log is opened as the supervisor's own is: against its folder, never through a link or a second name, private from its
    // creation (#556). `InvalidInput` / `PermissionDenied` when it refuses.
    let out = open_append_private(log)?;
    #[cfg(windows)]
    {
        winproc::spawn_detached(cmd, &out)
    }
    #[cfg(not(windows))]
    {
        cmd.stdin(std::process::Stdio::null())
            .stdout(out.try_clone()?)
            .stderr(out);
        #[cfg(unix)]
        unix::detach(cmd);
        let mut child = cmd.spawn()?;
        let pid = child.id();
        let _ = std::thread::Builder::new()
            .name("agora-reap".into())
            .spawn(move || {
                let _ = child.wait();
            });
        Ok(Detached { pid, in_job: false })
    }
}

/// A process started by [`spawn_detached`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Detached {
    pub pid: u32,
    /// Windows only: the caller's job forbade leaving it, so the process still ends when that job is closed.
    pub in_job: bool,
}

/// When process `pid` started, in a unit of the system's own (only compared for equality), or `None` when no such process runs (an
/// ended one that was not reaped yet counts as ended). A pid with the same start time is the same process, not a recycled pid: what
/// lock files compare. Where the start time cannot be read, a live process reports 0.
pub fn process_start_time(pid: u32) -> Option<u64> {
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    {
        procinfo::start_time(pid)
    }
    #[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
    {
        unix::alive(pid).then_some(0)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        None
    }
}

/// Asks process `pid` to end: `SIGTERM` on Unix, `TerminateProcess` on Windows.
pub fn terminate_process(pid: u32) -> io::Result<()> {
    // Pid 0 is no process of ours on any platform (on Unix it would signal the caller's whole process group).
    if pid == 0 {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    #[cfg(unix)]
    {
        unix::terminate(pid)
    }
    #[cfg(windows)]
    {
        winproc::terminate(pid)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// The variable through which [`spawn_bound`] tells a child (on Unix) which process it must end with; [`end_with_supervisor`] reads
/// and removes it.
const SUPERVISOR_VAR: &str = "AGORA_SUPERVISOR_PID";

/// Starts `cmd` bound to this process: the child ends when this process ends, however it ends (stopped, killed, crashed), so a
/// supervisor never leaves its child behind (ADR-0062 item 8).
///
/// Windows: the first call puts this process into a job of its own that ends every process in it when it is closed
/// (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`); only this process holds the job, so it closes when this process ends. The child starts
/// inside it, so there is no moment when it runs unbound. Processes in the job may still leave it on purpose
/// (`JOB_OBJECT_LIMIT_BREAKAWAY_OK`), as [`spawn_detached`] does. When this process may not join a job, the child starts unbound
/// and [`Bound::bound`] says so.
///
/// Unix: the child learns this process's id from its environment and must call [`end_with_supervisor`] first thing (Linux:
/// `PR_SET_PDEATHSIG`; macOS: a `kqueue` exit watch), which ends it with `SIGTERM` when this process ends. Spawn and wait from
/// the same thread, one that lives as long as the child: Linux ties the signal to the thread that started the child.
pub fn spawn_bound(cmd: &mut std::process::Command) -> io::Result<Bound> {
    #[cfg(windows)]
    {
        let bound = winproc::join_kill_on_close_job();
        // As `spawn_detached` started it before: no console of its own (a console-less parent's child would get a new one, a
        // window whose closing would end it) and out of any Ctrl-C group.
        std::os::windows::process::CommandExt::creation_flags(cmd, winproc::DETACHED_FLAGS);
        Ok(Bound {
            child: cmd.spawn()?,
            bound,
        })
    }
    #[cfg(not(windows))]
    {
        cmd.env(SUPERVISOR_VAR, std::process::id().to_string());
        Ok(Bound {
            child: cmd.spawn()?,
            bound: cfg!(any(target_os = "linux", target_os = "macos")),
        })
    }
}

/// Whether [`spawn_bound`] can bind a child here: a supervisor asks before starting anything, and refuses to start a child that could
/// outlive it. Windows: joins the job (once) and says whether that worked; Linux and macOS: yes; other systems: no.
pub fn children_can_be_bound() -> bool {
    #[cfg(windows)]
    {
        winproc::join_kill_on_close_job()
    }
    #[cfg(not(windows))]
    {
        cfg!(any(target_os = "linux", target_os = "macos"))
    }
}

/// A child started by [`spawn_bound`].
#[derive(Debug)]
pub struct Bound {
    pub child: std::process::Child,
    /// False when the child does not end with this process (Windows: this process could not join a job; a Unix system without a
    /// parent-exit watch).
    pub bound: bool,
}

/// In a child started by [`spawn_bound`]: arranges for this process to end when its supervisor ends, and removes the variable that
/// named it so nothing this process starts inherits it. Does nothing in a process started any other way. Call it at the very start
/// of `main`, before any other thread exists (it changes the environment).
pub fn end_with_supervisor() {
    let Some(value) = std::env::var_os(SUPERVISOR_VAR) else {
        return;
    };
    std::env::remove_var(SUPERVISOR_VAR);
    let parent = value.to_str().and_then(|v| v.parse::<u32>().ok());
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Some(parent) = parent {
        procinfo::end_with(parent);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let _ = parent;
}

/// Waits until `child` has ended without reaping it (Unix `waitid` with `WNOWAIT`): its pid stays its own, so a stop request passed
/// on by pid between this and the reaping [`std::process::Child::wait`] cannot reach a process that got the pid again. Windows
/// passes nothing on by pid and returns at once; the caller's `wait` does the waiting.
pub fn wait_unreaped(child: &std::process::Child) -> io::Result<()> {
    #[cfg(unix)]
    {
        unix::wait_unreaped(child.id())
    }
    #[cfg(not(unix))]
    {
        let _ = child;
        Ok(())
    }
}

/// The signal that ended a process, where there are signals (Unix); `None` for an exit with a code.
pub fn exit_signal(status: &std::process::ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        std::os::unix::process::ExitStatusExt::signal(status)
    }
    #[cfg(not(unix))]
    {
        let _ = status;
        None
    }
}

/// For a supervisor, and for `agora node`, which stops cleanly on them (ADR-0104 O2): from now on `SIGTERM`, `SIGINT` and `SIGHUP` do
/// not end this process; they set [`stop_requested`] and are passed on as `SIGTERM` to the process named by [`forward_stop_to`] (none
/// unless it is set). Windows has no such request for a detached process (`agora
/// down` ends it with `TerminateProcess`, and the job of [`spawn_bound`] takes the child with it): there this does nothing.
pub fn catch_stop_requests() -> io::Result<()> {
    #[cfg(unix)]
    {
        unix::catch_stop_requests()
    }
    #[cfg(not(unix))]
    {
        Ok(())
    }
}

/// [`catch_stop_requests`] for a process that stops cleanly on the first request (ADR-0104 O2: `agora node`): a signal that was ignored
/// when this is called stays ignored (`nohup`, a background job), and a second request ends the process at once, as the signal would
/// have without the catch. A request less than a second after the first is that first one reaching the process again, not a second
/// (#1119 item 4: Ctrl-C on a foreground `agora supervise node` signals the node and its supervisor, which passes its own on), and is
/// not acted on. Windows: the session's end (a log off, a shutdown, a restart) and the console's closing are the stop
/// request, and the system is held off while the process stops (a few seconds at most); `agora down` still ends the node outright.
pub fn catch_stop_requests_once() -> io::Result<()> {
    #[cfg(unix)]
    {
        unix::catch_stop_requests_once()
    }
    #[cfg(windows)]
    {
        windows::catch_session_end()
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(())
    }
}

/// The process a stop request caught by [`catch_stop_requests`] is passed on to (`None`: no one, between children).
pub fn forward_stop_to(pid: Option<u32>) {
    #[cfg(unix)]
    unix::forward_stop_to(pid);
    #[cfg(not(unix))]
    let _ = pid;
}

/// Whether [`catch_stop_requests_once`] catches anything here: Unix (the signals), and Windows (the session's end; `agora
/// down` ends a detached process there with `TerminateProcess`, which cannot be caught).
pub const STOP_REQUESTS_CAUGHT: bool = cfg!(any(unix, windows));

/// Ends this process as the stop request caught by [`catch_stop_requests`] would have ended it without the catch (Unix: that signal,
/// with its default action again; `SIGTERM` when none was caught), so whoever waits for it (a supervisor, a shell, a service manager)
/// sees the same end as before. For a process that stopped cleanly after the request (ADR-0104 O2: `agora node`). Other threads are not
/// waited for. Elsewhere: exit code 0.
pub fn end_as_stopped() -> ! {
    #[cfg(unix)]
    {
        unix::end_as_stopped()
    }
    #[cfg(not(unix))]
    {
        std::process::exit(0)
    }
}

/// Whether a stop request was caught since [`catch_stop_requests`] or [`catch_stop_requests_once`] (Windows: the session's end).
pub fn stop_requested() -> bool {
    #[cfg(unix)]
    {
        unix::stop_requested()
    }
    #[cfg(windows)]
    {
        windows::session_ending()
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

/// Opens `path` to append (creating it) for a best-effort log in a folder its caller already keeps private, such as the node's
/// `<data>/device-trust.log`: a new file is the user's only on Unix (`0o600`); on Windows it inherits the folder's ACL. Unlike
/// [`open_append_private`] it follows a link and accepts an existing file of any owner, so a data folder reached through a link, or
/// a log an elevated command created, keeps being written.
pub fn open_append_owner_created(path: &std::path::Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    unix::owner_only_on_create(&mut options);
    options.open(path)
}

/// Opens `path` to append for a log a process writes for itself (`agora supervise --log`), so that nothing planted in the Agora folder
/// can redirect what is written or change another file's permissions (`InvalidInput` when it refuses):
/// - the folder it is in is opened as itself, never through a link or junction, and the log is opened against that folder (Unix
///   `openat`; Windows: the folder is held so it can be neither renamed nor removed, and the log is checked to be inside it);
/// - a new log is created exclusively, readable and writable by the user only (never executable) from its creation on;
/// - an existing log is opened without following a link and refused when it is a link, not a regular file, has another name (a hard
///   link) or (Unix) belongs to another user; it is then made the user's only through its own handle.
///
/// The folders above the log's own (the Agora folder and its parents) are the user's choice and are not checked.
pub fn open_append_private(path: &std::path::Path) -> io::Result<std::fs::File> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    #[cfg(unix)]
    {
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        unix::open_private(dir, name, true)
    }
    #[cfg(windows)]
    {
        windows::open_private(dir, path, true)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = dir;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    }
}

/// Refusals: `InvalidInput` for a link, a second name or not a plain file or folder; `PermissionDenied` for one another user owns.
///
/// Opens (creating it) a lock file a process takes with `File::lock`, with every check of [`open_append_private`]: its folder opened as
/// itself, never through a link; a new file private from its creation (mode 0600, an owner-only ACL); an existing one a regular file
/// with one name and (Unix) this user's, made private through its handle. Read and write, never truncated. Another local user can then
/// neither open it to hold the lock nor plant a link that makes the lock land on another file. On Windows the file is not shared for
/// delete: while any process has it open it can be neither renamed nor removed ([`LOCK_FILES_MANDATORY`]).
pub fn open_lock_private(path: &std::path::Path) -> io::Result<std::fs::File> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    #[cfg(unix)]
    {
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        unix::open_private(dir, name, false)
    }
    #[cfg(windows)]
    {
        windows::open_private(dir, path, false)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = dir;
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
    }
}

/// Makes `dir` exist as a folder only its user can enter (Unix 0700; Windows an owner-only ACL inherited by what it holds), creating it
/// when missing. `InvalidInput` when it is a link or not a folder, or belongs to another user: a private folder is never reached
/// through a link, and is checked and changed through one handle of the folder itself.
pub fn ensure_private_dir(dir: &std::path::Path) -> io::Result<()> {
    if std::fs::symlink_metadata(dir).is_err_and(|e| e.kind() == io::ErrorKind::NotFound) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
        }
        #[cfg(not(unix))]
        std::fs::create_dir_all(dir)?;
    }
    // Checked and made private through one handle of the folder itself: never through a link, and only when it is this user's.
    #[cfg(unix)]
    {
        unix::ensure_private_dir(dir)
    }
    #[cfg(windows)]
    {
        windows::ensure_private_dir(dir)
    }
    #[cfg(not(any(unix, windows)))]
    {
        restrict_to_owner(dir)
    }
}

/// True where this build has a way to register a program to start at the user's login (ADR-0062 item 6): Windows (a logon task),
/// Linux (a systemd user unit) and macOS (a LaunchAgent). Whether this machine can use it is [`services_available`], asked at run time.
pub const SERVICES_SUPPORTED: bool = cfg!(any(windows, target_os = "linux", target_os = "macos"));

/// True where a login service starts only while the user is logged in unless the system is told otherwise, which Agora never does
/// itself (Linux: a systemd user manager runs from boot only with `loginctl enable-linger`). The caller says so after installing.
pub const SERVICES_NEED_LINGER_FOR_BOOT: bool = cfg!(target_os = "linux");

/// True where a program replaced by an update must be allowed onto the local network again before a login service can reach a hub
/// on another machine (macOS: Privacy & Security > Local Network keeps the permission per binary; measured with 0.3.12 → 0.3.13,
/// unsigned builds). The caller says so after an update when a service is registered.
pub const SERVICES_NEED_NETWORK_PERMISSION_AFTER_UPDATE: bool = cfg!(target_os = "macos");

/// Whether this machine can register a login service now: [`SERVICES_SUPPORTED`], and on Linux a `systemctl` (systemd's per-user
/// services) found on `PATH`, on macOS a login session (`launchctl print gui/<uid>`; none over SSH while nobody is logged in at the
/// screen). Asked at run time, not when Agora was built: one Linux binary runs with and without systemd.
pub fn services_available() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::services_available()
    }
    #[cfg(target_os = "macos")]
    {
        macservice::available()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        SERVICES_SUPPORTED
    }
}

/// Why a path or argument of a [`ServiceSpec`] cannot go into this system's service entry as it is (see [`service_refusal`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceRefusal {
    /// The system would read part of it as a variable (Windows expands `%VAR%` in a task's command line).
    Expands,
    /// Not valid Unicode, or a form the entry cannot carry (systemd drops spaces at the start or end of a folder).
    CannotCarry,
    /// A line break, a NUL or another control character (a systemd unit is read line by line).
    ControlCharacter,
    /// Not a full path: the program and the working directory must start at the root.
    NotAbsolute,
    /// A backslash in the working directory (systemd writes it unquoted, and a `\` at a line's end joins the next line).
    Backslash,
    /// A quote (`"`, `'`) or a backslash in the program's path: systemd refuses to run such an executable.
    ProgramCharacter,
}

/// The first path or argument of `spec` this system's service entry cannot carry as it is, and why; `None` when all can. The caller
/// asks before saying what it registers; [`service_install`] refuses the same ones with `InvalidInput`.
pub fn service_refusal(spec: &ServiceSpec) -> Option<(std::ffi::OsString, ServiceRefusal)> {
    #[cfg(target_os = "linux")]
    {
        linux::service_refusal(spec)
    }
    #[cfg(target_os = "macos")]
    {
        macservice::refusal(spec)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        // A Windows task: `%` expands, and every value must be Unicode.
        std::iter::once(spec.program.as_os_str())
            .chain(spec.args.iter().map(std::ffi::OsString::as_os_str))
            .find_map(|a| match a.to_str() {
                None => Some((a.to_owned(), ServiceRefusal::CannotCarry)),
                Some(t) if t.contains('%') => Some((a.to_owned(), ServiceRefusal::Expands)),
                Some(_) => None,
            })
    }
}

/// A service failure the caller words itself (carried inside the `io::Error` the `service_*` functions return; see
/// [`service_problem`]). Its English `Display` is for logs only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceProblem {
    /// The user's service manager cannot be reached from this session (systemd: no user bus, as in an SSH session without a user
    /// manager, or no systemd at all).
    NoUserManager,
    /// Something Agora did not write is at the service file's place (a link, a folder), and Agora does not replace it.
    InTheWay(PathBuf),
    /// A service of the same name is registered, but Agora did not write it (a unit made by hand): it is neither replaced, stopped
    /// nor removed. The path is its file, or its name when it is registered from elsewhere.
    NotOurs(PathBuf),
    /// Run as root (`sudo`): a login service is the user's own, and root would leave its files in the user's folder. Nothing is done.
    Root,
    /// Neither `XDG_CONFIG_HOME` nor `HOME` names a full path, so there is no folder for the user's service files.
    NoConfigFolder,
    /// After reloading, the service manager did not find the service file that was just written (it reads another folder).
    NotLoaded(PathBuf),
    /// The service manager would also read these files (drop-ins, or another file of the same name), which can change what the service
    /// runs; nothing was registered.
    Overridden(Vec<PathBuf>),
    /// Turned off by the user in the system's own settings (macOS: System Settings > General > Login Items & Extensions, "Allow in
    /// the Background"), which Agora cannot turn on: only the user can, there. Nothing was registered.
    TurnedOffInSettings,
}

impl std::fmt::Display for ServiceProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceProblem::NoUserManager => {
                f.write_str("the user's service manager cannot be reached")
            }
            ServiceProblem::InTheWay(p) => write!(f, "{} is not a plain file", p.display()),
            ServiceProblem::NotOurs(p) => {
                write!(f, "{} was not written by this program", p.display())
            }
            ServiceProblem::Root => f.write_str("refused as root"),
            ServiceProblem::NoConfigFolder => {
                f.write_str("neither XDG_CONFIG_HOME nor HOME is set")
            }
            ServiceProblem::NotLoaded(p) => write!(f, "{} was not loaded", p.display()),
            ServiceProblem::Overridden(ps) => write!(f, "also read: {ps:?}"),
            ServiceProblem::TurnedOffInSettings => f.write_str("turned off in the system settings"),
        }
    }
}

impl std::error::Error for ServiceProblem {}

/// Whether `agora service` may act in this process at all; an error with a [`ServiceProblem`] when not (Linux: run as root, or no
/// folder for the user's units; macOS: run as root). The caller asks before writing anything.
pub fn service_precheck() -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::service_precheck()
    }
    #[cfg(target_os = "macos")]
    {
        macservice::precheck()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Ok(())
    }
}

/// The files the service manager reads for the service `name` besides Agora's own (Linux: its drop-ins, and the unit file it loaded
/// when that is not Agora's), which can change what it runs. `status` and a repeated `install` name them; [`service_remove`] does not
/// delete them, and the caller names those still there afterwards. Empty elsewhere (launchd reads one file per job: a job of the
/// same label loaded from another file is [`service_foreign`]).
pub fn service_drop_ins(name: &str) -> io::Result<Vec<PathBuf>> {
    #[cfg(target_os = "linux")]
    {
        linux::service_drop_ins(name)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = name;
        Ok(Vec::new())
    }
}

/// Whether the service's file differs from what [`service_install`] would write now (Linux: the unit text; a changed setting of a newer
/// Agora). [`service_install`] then rewrites a file Agora wrote, and refuses one it did not. `false` elsewhere.
pub fn service_unit_differs(spec: &ServiceSpec) -> io::Result<bool> {
    #[cfg(target_os = "linux")]
    {
        linux::service_unit_differs(spec)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = spec;
        Ok(false)
    }
}

/// `Some(file)` when the service `name` is registered but was not written by Agora (Linux: a user unit of the same name made by hand
/// or by another tool): the caller leaves it alone, neither stopping, replacing nor removing it. `None` when it is Agora's own or not
/// registered (Windows: a task of the name without Agora's marker, named by its place in Task Scheduler, `\<name>`; macOS: a file at
/// the LaunchAgent's place without Agora's marker, or the job loaded from another file).
pub fn service_foreign(name: &str) -> io::Result<Option<PathBuf>> {
    #[cfg(target_os = "linux")]
    {
        linux::service_foreign(name)
    }
    #[cfg(target_os = "macos")]
    {
        macservice::foreign(name)
    }
    #[cfg(windows)]
    {
        winservice::foreign(name)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = name;
        Ok(None)
    }
}

/// The `PATH` a login service gets: the spec's, keeping the order but only full paths an entry can carry (an empty or relative entry
/// would mean "the folder the service manager runs it from", which is not the installing shell's; a control character or a Unicode
/// noncharacter cannot go into a unit or a property list). No other variable of the installing shell goes into the entry.
/// Shared by the systemd unit and the LaunchAgent (Windows applies no variables).
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn service_path_variable(env: &[(String, std::ffi::OsString)]) -> Option<String> {
    let (_, value) = env.iter().find(|(k, _)| k == "PATH")?;
    let kept: Vec<String> = std::env::split_paths(value)
        .filter(|p| p.is_absolute())
        .filter_map(|p| p.to_str().map(str::to_owned))
        .filter(|p| !p.chars().any(|c| c.is_control() || is_noncharacter(c)))
        .collect();
    (!kept.is_empty()).then(|| kept.join(":"))
}

/// A Unicode noncharacter (U+FDD0..U+FDEF, and the last two code points of every plane, U+FFFE and U+FFFF among them): XML 1.0
/// cannot carry U+FFFE and U+FFFF, and none of them is ever meant in a path.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn is_noncharacter(c: char) -> bool {
    let n = u32::from(c);
    (0xFDD0..=0xFDEF).contains(&n) || n & 0xFFFE == 0xFFFE
}

/// The [`ServiceProblem`] inside an error a `service_*` function returned, if it is one.
pub fn service_problem(e: &io::Error) -> Option<&ServiceProblem> {
    e.get_ref()?.downcast_ref::<ServiceProblem>()
}

/// A program registered to start when the user logs in, for this user only and without administrator rights (ADR-0062 item 6). The
/// caller (the `agora service` command) builds it; each system writes it in its own form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceSpec {
    /// The service's name in the system (a task, a unit, a launchd label): one name per role, from one constant of the caller.
    pub name: String,
    /// A sentence the system's own lists show.
    pub description: String,
    /// The program, by its full path.
    pub program: PathBuf,
    pub args: Vec<std::ffi::OsString>,
    /// Variables to set for it (the installing shell's `PATH` until ADR-0062 item 5 captures the login environment). Windows ignores
    /// them: a logon task gets the user's own environment.
    pub env: Vec<(String, std::ffi::OsString)>,
    /// Its working directory (the Agora folder); system files the registration needs for a moment are written there, owner-only.
    pub work_dir: PathBuf,
    /// Start the program again when it failed: killed by a signal or an exit with a non-zero code (systemd `Restart=on-failure`, launchd
    /// `KeepAlive = { SuccessfulExit = false }`). The supervisor exits 0 for every end it decided itself under a service (a give-up, a
    /// refusal, a clean stop), so only a crash of the supervisor itself is restarted. Windows has no such setting: ignored.
    pub restart_supervisor_on_failure: bool,
    /// Stopping or restarting the service ends only its supervisor (and, through it, the role); processes the role started in a session
    /// of their own (the terminal holder and its agents, ADR-0079) keep running. systemd: `KillMode=process` instead of
    /// `control-group`. macOS already leaves them (`AbandonProcessGroup` false, and each agent is in its own session, measured) and
    /// Windows has no holder yet: both ignore it.
    pub keep_children_on_stop: bool,
}

/// Whether a service is registered (see [`ServiceSpec`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceState {
    NotInstalled,
    /// Registered; `enabled` is false when it was turned off outside Agora (a disabled task, Login Items, a policy).
    Installed {
        enabled: bool,
    },
}

/// Registers `spec` to start at this user's login (it does not start it: [`service_start`]). `Unsupported` where
/// [`SERVICES_SUPPORTED`] is false; `InvalidInput` for a path or argument the system would change on the way or cannot carry (see
/// [`service_refusal`]). Linux: an owner-only systemd user unit, then `systemctl --user daemon-reload` and `enable`; a unit that is
/// already there with the same text is not written again. macOS: an owner-only LaunchAgent in `~/Library/LaunchAgents`, turned on
/// again (`launchctl enable`) when it was turned off.
pub fn service_install(spec: &ServiceSpec) -> io::Result<()> {
    #[cfg(windows)]
    {
        winservice::install(spec)
    }
    #[cfg(target_os = "linux")]
    {
        linux::service_install(spec)
    }
    #[cfg(target_os = "macos")]
    {
        macservice::install(spec)
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = spec;
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// Starts the registered service `name` now, as the system would at login.
pub fn service_start(name: &str) -> io::Result<()> {
    #[cfg(windows)]
    {
        winservice::start(name)
    }
    #[cfg(target_os = "linux")]
    {
        linux::service_start(name)
    }
    #[cfg(target_os = "macos")]
    {
        macservice::start(name)
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = name;
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// Unregisters the service `name`; `Ok(false)` when it was not registered. It does not stop what the service started: the caller
/// stops the role through its lock first (ending a Windows task ends only its `conhost`).
pub fn service_remove(name: &str) -> io::Result<bool> {
    #[cfg(windows)]
    {
        winservice::remove(name)
    }
    #[cfg(target_os = "linux")]
    {
        linux::service_remove(name)
    }
    #[cfg(target_os = "macos")]
    {
        macservice::remove(name)
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = name;
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// Whether the service `name` is registered, and enabled. An error when the system could not say (access denied, a broken scheduler,
/// no user service manager in this session: [`ServiceProblem::NoUserManager`]): never reported as "not registered".
pub fn service_status(name: &str) -> io::Result<ServiceState> {
    #[cfg(windows)]
    {
        winservice::status(name)
    }
    #[cfg(target_os = "linux")]
    {
        linux::service_status(name)
    }
    #[cfg(target_os = "macos")]
    {
        macservice::status(name)
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = name;
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// Shows one notification of the system's own, with `title` and `body` (ADR-0048 decision 5: the desktop shell tells the user that
/// approvals wait while its window is out of sight). No library: the system's own notifier is run by its full path with a fixed
/// argument list, and the words travel as separate arguments (Linux `notify-send`) or as
/// environment variables (Windows PowerShell), never inside a script, so no text can change what runs. The words come from
/// `locales/` (the caller's). Waits for the notifier; `NotFound` where there is none (Linux without `notify-send`),
/// `PermissionDenied` where the user turned notifications off (Windows), `Unsupported` on macOS for now (see `macos.rs`).
pub fn desktop_notify(title: &str, body: &str) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        macos::desktop_notify(title, body)
    }
    #[cfg(target_os = "linux")]
    {
        linux::desktop_notify(title, body)
    }
    #[cfg(windows)]
    {
        winservice::desktop_notify(title, body)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = (title, body);
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// This machine's name (`gethostname` on Unix, `COMPUTERNAME` on Windows), if it has one.
pub fn hostname() -> Option<String> {
    #[cfg(unix)]
    {
        unix::hostname()
    }
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME")
            .ok()
            .filter(|n| !n.trim().is_empty())
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

/// One address of one network interface of this machine (see [`interface_addresses`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceAddr {
    /// The interface's name as the system gives it (`eth0`, `en0`; an adapter GUID on Windows). For messages only.
    pub name: String,
    pub ip: std::net::IpAddr,
    /// The length of the network prefix (24 for 192.168.1.20/24). 0 when the system gave none.
    pub prefix_len: u8,
    /// The interface is up.
    pub up: bool,
    pub loopback: bool,
    /// An IPv6 address the system marks temporary (privacy address) or deprecated: it changes or goes away on its own, so it is not
    /// an address to name in a certificate. Known on Linux (`/proc/net/if_inet6`) and Windows (suffix origin and lifetime); macOS does
    /// not say through `getifaddrs`, so it is `false` there.
    pub transient: bool,
}

/// Every unicast IPv4 and IPv6 address of this machine's network interfaces, up or down (std has no way to list them). The caller
/// decides which are worth using. An error means the system refused to list them (the caller must not guess). Linux and macOS:
/// `getifaddrs`; Windows: `GetAdaptersAddresses`.
pub fn interface_addresses() -> io::Result<Vec<InterfaceAddr>> {
    #[cfg(unix)]
    {
        unix::interface_addresses()
    }
    #[cfg(windows)]
    {
        windows::interface_addresses()
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "listing network interfaces is not supported on this system",
        ))
    }
}

/// A TCP listener on `127.0.0.1:port` (0 picks a free port) that no other socket, of this user or another, can bind beside while it
/// is open: `SO_EXCLUSIVEADDRUSE` on Windows (where a `SO_REUSEADDR` binder could otherwise share it), and on Linux and macOS a socket
/// without `SO_REUSEPORT` (checked, not assumed). For a fixed, predictable loopback port such as the panel's (threat model T27). It
/// does not keep the port while nothing is bound to it: that is the caller's to say.
pub fn bind_loopback_exclusive(port: u16) -> io::Result<std::net::TcpListener> {
    #[cfg(unix)]
    {
        unix::bind_loopback_exclusive(port)
    }
    #[cfg(windows)]
    {
        windows::bind_loopback_exclusive(port)
    }
    #[cfg(not(any(unix, windows)))]
    {
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
    }
}

/// Whether `url` may be handed to the system's opener: `http://` or `https://` (any case), a host, no `@` in the host part (no
/// credentials), at most 2048 bytes, and only the bytes of `url_byte_ok` (RFC 3986 characters minus `^ " ( ) ! < > |`, with `%` only before two hex digits). Platform-neutral
/// so it is tested everywhere.
pub fn is_openable_url(url: &str) -> bool {
    if url.is_empty() || url.len() > 2048 {
        return false;
    }
    if !url_bytes_ok(url) {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    let Some(rest) = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    !authority.is_empty() && !authority.contains('@') && !authority.starts_with(':')
}

/// The bytes an address may hold: the RFC 3986 unreserved and reserved characters, minus `% ^ " ( ) ! < > |` (`cmd.exe` and shells
/// act on those; `%` is allowed only as a percent-encoding (`%` and two hex digits, see `url_bytes_ok`); the Windows developer measured that `rundll32` leaves `%NAME%` literal, but a `%NAME%` shape is still refused as defence in depth; `&` is allowed: no shell and no `cmd.exe` reads the address, it is one argument to `rundll32` on Windows and `open` and `xdg-open` are exec'd directly, and links with a query string need it). Letters and digits, and `-._~:/?#[]@$'*+,;=&`.
/// Nothing else: no space, control character, backtick, brace, backslash or non-ASCII byte.
fn url_byte_ok(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-._~:/?#[]@$'*+,;=&".contains(&b)
}

/// Whether every byte of `url` is allowed: the bytes of `url_byte_ok`, and a `%` only as the start of a percent-encoding (`%` and two hex
/// digits). So `%20` passes, while `%NAME%`, `%1` and a lone `%` do not.
fn url_bytes_ok(url: &str) -> bool {
    let b = url.as_bytes();
    b.iter().enumerate().all(|(i, &c)| {
        if c == b'%' {
            b.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit)
        } else {
            url_byte_ok(c)
        }
    })
}

/// Where `rundll32.exe` is: under `system_root` (the `SystemRoot` of the environment this process was started in) `\System32`, or
/// the default Windows folder when there is none; never found through `PATH`.
#[cfg_attr(not(windows), allow(dead_code))]
fn rundll32_path(system_root: Option<&str>) -> String {
    let root = system_root
        .map(|r| r.trim_end_matches(['\\', '/']))
        .filter(|r| !r.is_empty())
        .unwrap_or("C:\\Windows");
    format!("{root}\\System32\\rundll32.exe")
}

/// Opens `url` in the user's browser: `open` on macOS, `rundll32 url.dll,FileProtocolHandler` on Windows (the address is one argument
/// of a program, never read by `cmd.exe`), `xdg-open` elsewhere. A `url` that [`is_openable_url`] refuses is not passed to anything
/// (`InvalidInput`). An error means no browser could be asked (the caller then shows the address); a helper still running after a few
/// seconds counts as success.
pub fn open_url(url: &str) -> io::Result<()> {
    use std::process::{Command, Stdio};
    if !is_openable_url(url) {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    #[cfg(target_os = "macos")]
    let mut cmd = Command::new("open");
    #[cfg(windows)]
    let mut cmd = {
        let root = std::env::var("SystemRoot").ok();
        let mut c = Command::new(rundll32_path(root.as_deref()));
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let mut cmd = Command::new("xdg-open");
    let mut child = cmd
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err(io::Error::other("no browser opened the address"))
            };
        }
        if std::time::Instant::now() > end {
            // Still working (some helpers wait for the browser): leave it be, and reap it from a thread.
            let _ = std::thread::Builder::new().spawn(move || {
                let _ = child.wait();
            });
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod open_url_tests {
    use super::{is_openable_url, open_url, rundll32_path};

    #[test]
    fn rundll32_is_found_under_the_system_root_never_through_the_path() {
        assert_eq!(
            rundll32_path(Some("D:\\WINNT")),
            "D:\\WINNT\\System32\\rundll32.exe"
        );
        assert_eq!(
            rundll32_path(Some("C:\\Windows\\")),
            "C:\\Windows\\System32\\rundll32.exe"
        );
        assert_eq!(rundll32_path(None), "C:\\Windows\\System32\\rundll32.exe");
        assert_eq!(
            rundll32_path(Some("")),
            "C:\\Windows\\System32\\rundll32.exe"
        );
    }

    #[test]
    fn only_plain_web_addresses_may_be_opened() {
        for ok in [
            "http://a.test",
            "https://a.test/x?y=1#z",
            "HTTPS://A.TEST/",
            "https://localhost:8443/#code=abc123",
            "http://127.0.0.1:8080/p",
            "https://a.test/?x=1&y=2&calc.exe",
            "https://a.test/a%20b%E2%80%93c",
        ] {
            assert!(is_openable_url(ok), "{ok}");
        }
        for bad in [
            "",
            "javascript:alert(1)",
            "javascript:a.test/x",
            "file:///etc/passwd",
            "ftp://a.test",
            "calc.exe",
            "https://",
            "https://user:pw@a.test/",
            "https://a.test/%GITHUB_TOKEN%",
            "https://a.test/%4",
            "https://a.test/%41%",
            "https://a.test/%zz",
            "https://a.test/^x",
            "https://a.test/\"x",
            "https://a.test/a(b)",
            "https://a.test/!x",
            "https://a.test/<x>",
            "https://a.test/a|b",
            "https://a.test/a b",
            "https://a.test/a\tb",
            "https://a.test/a\nb",
            "https://a.test/\u{e9}",
            "https://a.test/`x`",
            "https://a.test/{x}",
            "https://a.test/\\x",
            "https://a.test/\u{7f}",
        ] {
            assert!(!is_openable_url(bad), "{bad:?}");
        }
        assert!(!is_openable_url(&format!(
            "https://a.test/{}",
            "x".repeat(2048)
        )));
    }

    #[test]
    fn a_refused_address_is_never_handed_to_a_program() {
        let e = open_url("https://a.test/?x=%TOKEN%").unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod detached_tests {
    #[test]
    fn a_live_process_keeps_its_start_time_and_an_ended_one_has_none() {
        let me = std::process::id();
        let first = super::process_start_time(me);
        assert!(first.is_some());
        assert_eq!(super::process_start_time(me), first);
        if !cfg!(any(unix, windows)) {
            return;
        }
        let exe = std::env::current_exe().unwrap();
        let mut child = std::process::Command::new(exe)
            .arg("--list")
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert_eq!(super::process_start_time(pid), None);
        assert!(super::hostname().is_some_and(|h| !h.is_empty()));
    }
}

/// The process and all its descendants, parents before children. Processes that exit while the tree is
/// being walked are skipped.
pub fn process_tree(root: u32) -> io::Result<Vec<ProcessInfo>> {
    walk_tree(root, process_name, process_children)
}

/// [`process_tree`] over any process table. Each process is visited once: on Windows a process keeps the id of a parent that ended,
/// and ids are reused, so a stale parent id can name one of the root's own descendants. The table then has a cycle, and a walk
/// without this check never ended (a Windows CI run hung in `a_child_is_found_under_its_parent_and_in_the_tree`; the node walks
/// the same tree on every tick).
fn walk_tree(
    root: u32,
    mut name_of: impl FnMut(u32) -> io::Result<String>,
    mut children_of: impl FnMut(u32) -> io::Result<Vec<u32>>,
) -> io::Result<Vec<ProcessInfo>> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::from([root]);
    let mut queue = std::collections::VecDeque::from([(root, 0u32)]);
    while let Some((pid, ppid)) = queue.pop_front() {
        let Ok(name) = name_of(pid) else {
            if pid == root {
                return Err(io::Error::new(io::ErrorKind::NotFound, "process not found"));
            }
            continue;
        };
        out.push(ProcessInfo { pid, ppid, name });
        if let Ok(kids) = children_of(pid) {
            queue.extend(
                kids.into_iter()
                    .filter(|k| seen.insert(*k))
                    .map(|k| (k, pid)),
            );
        }
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod walk_tree_tests {
    use super::*;

    #[test]
    fn a_cycle_in_the_process_table_is_walked_once() {
        // 10 → 11 → 12, and 12 is listed as the parent of 10 (a stale parent id reused by a descendant), and 12 of itself again
        // through 13 → 11.
        let table: &[(u32, &[u32])] = &[(10, &[11]), (11, &[12]), (12, &[10, 13]), (13, &[11])];
        let children = |pid: u32| {
            Ok(table
                .iter()
                .find(|(p, _)| *p == pid)
                .map_or(Vec::new(), |(_, k)| k.to_vec()))
        };
        let tree = walk_tree(10, |pid| Ok(format!("p{pid}")), children).unwrap();
        let pids: Vec<u32> = tree.iter().map(|p| p.pid).collect();
        assert_eq!(pids, [10, 11, 12, 13]);
        assert_eq!(tree[0].ppid, 0);
        assert_eq!(tree[3].ppid, 12);
    }

    #[test]
    fn a_child_that_ended_during_the_walk_is_skipped_and_a_missing_root_is_an_error() {
        let children = |pid: u32| Ok(if pid == 1 { vec![2, 3] } else { vec![] });
        let name = |pid: u32| {
            if pid == 2 {
                Err(io::Error::from(io::ErrorKind::NotFound))
            } else {
                Ok(String::from("x"))
            }
        };
        let pids: Vec<u32> = walk_tree(1, name, children)
            .unwrap()
            .iter()
            .map(|p| p.pid)
            .collect();
        assert_eq!(pids, [1, 3]);
        assert!(walk_tree(2, name, children).is_err());
    }
}

/// Start a process in a new PTY.
pub fn spawn(config: &PtyConfig) -> io::Result<Box<dyn Pty>> {
    spawn_checked(config, None)
}

/// [`spawn`] in the folder `folder` holds; `config.cwd` is not where it starts: when given it is the caller's spelling of that folder's
/// path (it may name it through a link), and on Linux confinement entries at or below either spelling are made from the held folder.
/// Refused ([`folder_moved`]) when, with everything else made for the start (the confinement too), the path no longer names that
/// folder. Unix: the process is moved into the held folder itself (`fchdir` on the open handle) before it runs, so a swap even after
/// that last check is not followed. Windows: it is started by the path, which the handle keeps from being renamed or removed.
pub fn spawn_in(config: &PtyConfig, folder: &HeldFolder) -> io::Result<Box<dyn Pty>> {
    spawn_checked(config, Some(folder))
}

fn spawn_checked(config: &PtyConfig, folder: Option<&HeldFolder>) -> io::Result<Box<dyn Pty>> {
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
        unix::spawn_in(config, folder)
    }
    #[cfg(windows)]
    {
        match folder {
            Some(f) => {
                let mut config = config.clone();
                config.cwd = Some(f.path.clone());
                if !f.in_place() {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, FolderMoved));
                }
                conpty::spawn(&config)
            }
            None => conpty::spawn(config),
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)] // expect is allowed in tests
mod list_dir_tests {
    struct Folder(std::path::PathBuf);

    impl Folder {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            loop {
                let path = std::env::temp_dir().join(format!(
                    "agora-visitdir-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match std::fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => panic!("create fixture: {e}"),
                }
            }
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn listing_keeps_the_open_folder_after_its_path_is_replaced() {
        let folder = Folder::new();
        let original = folder.0.join("original");
        let moved = folder.0.join("moved");
        let outside = folder.0.join("outside");
        std::fs::create_dir(&original).expect("original");
        std::fs::create_dir(&outside).expect("outside");
        std::fs::write(original.join("own.txt"), "own").expect("own file");
        std::fs::write(outside.join("outside.txt"), "outside").expect("outside file");
        let handle = super::open_dir(&original).expect("open original");
        std::fs::rename(&original, &moved).expect("move original");
        #[cfg(windows)]
        {
            // Junctions need no symbolic-link privilege: this coverage must run on an ordinary Windows CI account.
            let result = std::process::Command::new("cmd.exe")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(&original)
                .arg(&outside)
                .output()
                .expect("junction command");
            assert!(result.status.success(), "{result:?}");
        }
        #[cfg(not(windows))]
        super::symlink(&outside, &original).expect("replace path");
        let items = super::list_dir(&handle).expect("held directory");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "own.txt");
        let replaced = super::open_dir(&original).expect("replacement directory");
        let items = super::list_dir(&replaced).expect("replacement listing");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "outside.txt");
    }

    #[test]
    fn visiting_a_folder_reads_metadata_lazily_and_stops_early() {
        use std::ops::ControlFlow;
        let folder = Folder::new();
        let names = ["a", "b", "c"];
        for name in names {
            std::fs::write(folder.0.join(name), "x").expect("file");
        }
        let handle = super::open_dir(&folder.0).expect("open");
        let mut visited = 0;
        super::visit_dir(&handle, |item| {
            if visited == 0 {
                assert_eq!(item.len, 1);
                // Directory order is irrelevant: change every other file after the first callback.
                // An eager Vec would already contain stale metadata for those entries.
                for name in names {
                    if item.name != name {
                        std::fs::write(folder.0.join(name), "changed").expect("update");
                    }
                }
            } else {
                assert_eq!(
                    item.len, 7,
                    "later metadata was read before the first callback"
                );
            }
            visited += 1;
            ControlFlow::Continue(())
        })
        .expect("visit");
        assert_eq!(visited, names.len());

        let mut stopped = 0;
        super::visit_dir(&handle, |_| {
            stopped += 1;
            ControlFlow::Break(())
        })
        .expect("stop");
        assert_eq!(stopped, 1);
        // Early return closes only the enumeration's duplicate; the caller can list the handle again.
        assert_eq!(
            super::list_dir(&handle).expect("list after stop").len(),
            names.len()
        );
    }

    #[test]
    fn a_folder_lists_through_its_handle_without_following_links() {
        let d = std::env::temp_dir().join(format!("agora-listdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).expect("dir");
        std::fs::write(d.join("f.txt"), "abc").expect("file");
        let linked = super::symlink(&d.join("sub"), &d.join("link")).is_ok();
        let handle = super::open_dir(&d).expect("open");
        let mut items = super::list_dir(&handle).expect("list");
        items.sort_by(|a, b| a.name.cmp(&b.name));
        let names: Vec<_> = items
            .iter()
            .map(|i| i.name.to_string_lossy().into_owned())
            .collect();
        let mut want = vec!["f.txt", "sub"];
        if linked {
            want.insert(1, "link");
        }
        assert_eq!(names, want);
        let get = |n: &str| items.iter().find(|i| i.name == n).expect("item");
        assert!(get("sub").is_dir && !get("f.txt").is_dir);
        assert_eq!(get("f.txt").len, 3);
        if linked {
            assert!(!get("link").is_dir, "a link to a folder is not a folder");
        }
        // Listing twice through the same handle gives the same entries.
        assert_eq!(super::list_dir(&handle).expect("again").len(), items.len());
        // A file is not a folder.
        assert!(super::open_dir(&d.join("f.txt")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)] // expect is allowed in tests
mod opened_path_tests {
    #[test]
    fn an_open_file_reports_where_it_really_is_even_through_a_link() {
        let d = std::env::temp_dir().join(format!("agora-opened-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("real")).expect("dir");
        std::fs::write(d.join("real/f.txt"), "x").expect("file");
        let real = std::fs::canonicalize(d.join("real/f.txt")).expect("canonical");
        let direct = std::fs::File::open(d.join("real/f.txt")).expect("open");
        assert_eq!(super::opened_path(&direct).expect("path"), real);
        if super::symlink(&d.join("real"), &d.join("link")).is_ok() {
            let through = std::fs::File::open(d.join("link/f.txt")).expect("open");
            assert_eq!(super::opened_path(&through).expect("path"), real);
        }
        let _ = std::fs::remove_dir_all(&d);
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod secret_tests {
    use super::secret_from;
    use std::io::ErrorKind;

    fn read(bytes: &[u8]) -> std::io::Result<String> {
        secret_from(&mut &bytes[..])
    }

    #[test]
    fn a_secret_line_is_edited_here_and_ctrl_c_stops_it() {
        assert_eq!(read(b"open sesame\r").unwrap(), "open sesame");
        assert_eq!(read(b"abx\x7fc\n").unwrap(), "abc");
        assert_eq!(read("güz\u{e7}\x7f\x7fel\r".as_bytes()).unwrap(), "güel");
        assert_eq!(read(b"junk\x15ok\r").unwrap(), "ok");
        assert_eq!(read(b"a\x1bb\r").unwrap(), "ab");
        assert_eq!(
            read(b"secr\x03et\r").unwrap_err().kind(),
            ErrorKind::Interrupted
        );
        assert_eq!(read(b"\x04").unwrap_err().kind(), ErrorKind::UnexpectedEof);
        assert_eq!(read(b"").unwrap_err().kind(), ErrorKind::UnexpectedEof);
        assert_eq!(read(b"\xff\r").unwrap_err().kind(), ErrorKind::InvalidData);
    }

    use super::{confirmation_from, Confirmation};
    use std::time::{Duration, Instant};

    /// A terminal that gives `keys` one wait after another (`None` is a wait in which no key came), then no more keys.
    fn answer(
        keys: Vec<Option<&'static [u8]>>,
        min_wait: Duration,
        stop_after: Option<usize>,
    ) -> (std::io::Result<Confirmation>, usize) {
        let mut keys = keys.into_iter();
        let mut key = move |_: Duration| -> std::io::Result<Option<Vec<u8>>> {
            Ok(keys.next().flatten().map(<[u8]>::to_vec))
        };
        let mut asked = 0;
        let mut stop = || {
            asked += 1;
            stop_after.is_some_and(|n| asked > n)
        };
        let got = confirmation_from(&mut key, Instant::now(), min_wait, &mut stop);
        (got, asked)
    }

    /// #942: the owner's question at `agora pair open` ends when it was answered in a window, before or while a line is typed; what
    /// was typed then counts for nothing. Otherwise the rules of `read_confirmation` hold.
    #[test]
    fn a_confirmation_ends_when_the_caller_says_it_was_answered_elsewhere() {
        let z = Duration::ZERO;
        // A line, with waits in which nothing came; the caller is asked before every wait.
        let (got, asked) = answer(vec![None, Some(b"y"), None, Some(b"\r")], z, None);
        assert_eq!(got.unwrap(), Confirmation::Line("y".into()));
        assert_eq!(asked, 4);
        // Several keys in one read.
        let (got, _) = answer(vec![Some(b"yes\rmore")], z, None);
        assert_eq!(got.unwrap(), Confirmation::Line("yes".into()));
        // Answered elsewhere before any key, and in the middle of a line.
        let (got, asked) = answer(vec![None, None, Some(b"y\r")], z, Some(2));
        assert_eq!(got.unwrap(), Confirmation::Stopped);
        assert_eq!(asked, 3);
        let (got, _) = answer(vec![Some(b"y"), None, Some(b"\r")], z, Some(2));
        assert_eq!(got.unwrap(), Confirmation::Stopped);
        // A first key before the minimum wait is not an answer, and nothing more is read.
        let (got, asked) = answer(vec![Some(b"y\r")], Duration::from_secs(3600), None);
        assert_eq!(got.unwrap(), Confirmation::TooSoon);
        assert_eq!(asked, 1);
        // The end of input and Ctrl-C are not answers, and a line stops at 256 bytes.
        let (got, _) = answer(vec![Some(b"")], z, None);
        assert_eq!(got.unwrap_err().kind(), ErrorKind::UnexpectedEof);
        let (got, _) = answer(vec![Some(b"y\x03\r")], z, None);
        assert_eq!(got.unwrap_err().kind(), ErrorKind::Interrupted);
        let long: &'static [u8] = Box::leak(vec![b'y'; 300].into_boxed_slice());
        let (got, _) = answer(vec![Some(long), Some(b"\r")], z, None);
        assert_eq!(got.unwrap(), Confirmation::Line("y".repeat(256)));
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used)]
mod process_group_tests {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    /// The state letters `ps` gives for a process, empty when there is none.
    fn state(pid: &str) -> String {
        let out = Command::new("ps")
            .args(["-o", "stat=", "-p", pid])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn dead(pid: &str) -> bool {
        let end = Instant::now() + Duration::from_secs(3);
        loop {
            let s = state(pid);
            if s.is_empty() || s.starts_with('Z') {
                return true;
            }
            if Instant::now() > end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn the_child_leads_a_group_of_its_own_and_killing_it_ends_what_it_started() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 60 & echo $!; wait"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped());
        super::own_process_group(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        let mut out = child.stdout.take().unwrap();
        let mut pid = String::new();
        let mut b = [0u8; 1];
        while out.read(&mut b).unwrap() == 1 && b[0] != b'\n' {
            pid.push(b[0] as char);
        }
        assert!(!dead_now(&pid), "the background process runs");
        // The child is the leader of its own group (not ours).
        let group = Command::new("ps")
            .args(["-o", "pgid=", "-p", &child.id().to_string()])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&group.stdout).trim(),
            child.id().to_string()
        );
        super::kill_process_tree(&mut child);
        assert!(dead(&pid), "the process the child started is ended with it");
        assert!(child.try_wait().unwrap().is_some(), "the child is reaped");
    }

    fn dead_now(pid: &str) -> bool {
        let s = state(pid);
        s.is_empty() || s.starts_with('Z')
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod rename_no_replace_tests {
    use super::rename_no_replace;
    use std::io::ErrorKind;

    /// #1150: a name that is taken (a file, a folder, a link to nothing) is refused and left as it was, and the file stays under its
    /// old name; a free name takes it in one step.
    #[test]
    fn a_taken_name_is_refused_and_never_replaced() {
        let dir = std::env::temp_dir().join(format!("agora-rename-excl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        let (from, taken) = (dir.join("new"), dir.join("taken"));
        std::fs::write(&from, b"new").unwrap();
        std::fs::write(&taken, b"old").unwrap();
        match rename_no_replace(&from, &taken) {
            // A file system without it: nothing was moved.
            Err(e) if e.kind() == ErrorKind::Unsupported => {
                assert_eq!(std::fs::read(&from).unwrap(), b"new");
                assert_eq!(std::fs::read(&taken).unwrap(), b"old");
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
            Err(e) => assert_eq!(e.kind(), ErrorKind::AlreadyExists, "{e}"),
            Ok(()) => panic!("replaced a file"),
        }
        assert_eq!(std::fs::read(&taken).unwrap(), b"old");
        assert_eq!(std::fs::read(&from).unwrap(), b"new");
        let folder = rename_no_replace(&from, &dir.join("folder")).unwrap_err();
        assert_eq!(folder.kind(), ErrorKind::AlreadyExists, "{folder}");
        assert!(dir.join("folder").is_dir());
        #[cfg(unix)]
        {
            let dangling = dir.join("dangling");
            std::os::unix::fs::symlink(dir.join("nowhere"), &dangling).unwrap();
            let e = rename_no_replace(&from, &dangling).unwrap_err();
            assert_eq!(e.kind(), ErrorKind::AlreadyExists, "{e}");
            assert!(!dir.join("nowhere").exists(), "the link is not followed");
            assert!(std::fs::symlink_metadata(&dangling)
                .unwrap()
                .file_type()
                .is_symlink());
        }
        let free = dir.join("free");
        rename_no_replace(&from, &free).unwrap();
        assert!(!from.exists());
        assert_eq!(std::fs::read(&free).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// ADR-0085 (Tests): a certificate renewal replaces `cert.pem` and `key.pem` with [`write_private`], which renames a temporary file over
/// the existing one. On Unix that is one atomic step; on Windows it is the same rename over an existing file. These tests run on
/// Linux, macOS and Windows in CI.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod atomic_replace_tests {
    use super::{set_aside, sync_parent_dir, write_private};

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("agora-plat-repl-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_file_that_exists_is_replaced_whole_each_time_and_no_temporary_is_left() {
        let d = dir("over");
        let cert = d.join("cert.pem");
        let key = d.join("key.pem");
        for round in 0..5 {
            let c = format!("certificate {round} {}", "c".repeat(round * 700));
            let k = format!("key {round} {}", "k".repeat(round * 300));
            write_private(&cert, c.as_bytes()).unwrap();
            write_private(&key, k.as_bytes()).unwrap();
            assert_eq!(std::fs::read_to_string(&cert).unwrap(), c);
            assert_eq!(std::fs::read_to_string(&key).unwrap(), k);
        }
        let mut names: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["cert.pem", "key.pem"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_replace_that_cannot_happen_leaves_what_was_there_untouched_and_no_temporary() {
        let d = dir("fail");
        // A folder where the file should go: the rename over it fails on every platform, as a full disk or a vanished folder would.
        let target = d.join("cert.pem");
        std::fs::create_dir_all(target.join("inside")).unwrap();
        assert!(write_private(&target, b"new").is_err());
        assert!(target.join("inside").is_dir());
        assert_eq!(
            std::fs::read_dir(&d).unwrap().count(),
            1,
            "a temporary file stayed behind"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Shutdown safety: a process killed in the middle of a replace (here: its temporary file written half, as a kill or a power cut leaves
    /// it) leaves the old file whole, and the next replace is not stopped by what was left.
    #[test]
    fn a_replace_killed_half_way_leaves_the_old_file_whole_and_the_next_one_goes_through() {
        let d = dir("killed");
        let ledger = d.join("ledger.json");
        write_private(&ledger, b"{\"entries\":[1,2,3]}").unwrap();
        // What an earlier process with this pid left under the names this process takes next (a restarted container's first
        // process has the same pid and counts from 0 again). The margin covers other tests of this run taking names meanwhile.
        let next = super::NEXT_TMP.load(std::sync::atomic::Ordering::Relaxed);
        let left: Vec<std::path::PathBuf> = (next..next + 1024)
            .map(|seq| {
                let mut name = ledger.as_os_str().to_owned();
                name.push(format!(".tmp-{}-{seq}", std::process::id()));
                std::path::PathBuf::from(name)
            })
            .collect();
        for l in &left {
            std::fs::write(l, b"{\"entries\":[1,2").unwrap();
        }
        assert_eq!(std::fs::read(&ledger).unwrap(), b"{\"entries\":[1,2,3]}");
        write_private(&ledger, b"{\"entries\":[4]}").unwrap();
        assert_eq!(std::fs::read(&ledger).unwrap(), b"{\"entries\":[4]}");
        // The name the replace took was cleared and renamed over the file; the others are still there.
        assert_eq!(left.iter().filter(|l| l.exists()).count(), left.len() - 1);
        sync_parent_dir(&ledger);
        sync_parent_dir(std::path::Path::new("relative-name-in-the-current-folder"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Shutdown safety: a saved file that cannot be read is moved aside, never deleted and never over another one set aside before.
    #[test]
    fn a_file_set_aside_keeps_its_bytes_and_never_replaces_an_earlier_one() {
        let d = dir("aside");
        let file = d.join("events.log");
        std::fs::write(&file, b"one").unwrap();
        let first = set_aside(&file, "unreadable-7").unwrap();
        assert_eq!(first, d.join("events.log.unreadable-7"));
        assert!(!file.exists());
        std::fs::write(&file, b"two").unwrap();
        let second = set_aside(&file, "unreadable-7").unwrap();
        assert_eq!(second, d.join("events.log.unreadable-7-2"));
        assert_eq!(std::fs::read(&first).unwrap(), b"one");
        assert_eq!(std::fs::read(&second).unwrap(), b"two");
        // A tag is reduced to a plain name: it never leaves the folder.
        std::fs::write(&file, b"three").unwrap();
        let third = set_aside(&file, "../x y").unwrap();
        assert_eq!(third, d.join("events.log..._x_y"));
        assert_eq!(
            set_aside(&d.join("missing"), "x").unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod read_timeout_tests {
    use super::*;

    fn never_asked() -> io::Result<bool> {
        panic!("the hang-up is checked only for an InvalidInput answer")
    }

    #[test]
    fn invalid_input_on_a_hung_up_stream_is_the_peer_closed_answer_and_others_are_errors() {
        assert_eq!(
            read_timeout_result(Ok(()), never_asked).unwrap(),
            ReadTimeout::Set
        );
        let invalid = || Err(io::Error::from(io::ErrorKind::InvalidInput));
        let closed = read_timeout_result(invalid(), || Ok(true));
        assert_eq!(closed.unwrap(), ReadTimeout::PeerClosed);
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::BrokenPipe,
            io::ErrorKind::Other,
        ] {
            let e = read_timeout_result(Err(io::Error::from(kind)), never_asked).unwrap_err();
            assert_eq!(e.kind(), kind);
        }
    }

    /// Issue #830: an `InvalidInput` answer on a stream that is still open (or whose state cannot be read) is a failure, not a
    /// closed peer.
    #[test]
    fn invalid_input_on_an_open_stream_is_an_error() {
        let invalid = || Err(io::Error::from(io::ErrorKind::InvalidInput));
        let open = read_timeout_result(invalid(), || Ok(false)).unwrap_err();
        assert_eq!(open.kind(), io::ErrorKind::InvalidInput);
        let unknown = read_timeout_result(invalid(), || Err(io::Error::from(io::ErrorKind::Other)))
            .unwrap_err();
        assert_eq!(unknown.kind(), io::ErrorKind::InvalidInput);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod accept_timeout_tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn listener(name: &str) -> (LocalListener, PathBuf) {
        // Short: Unix socket paths are limited to about 100 bytes.
        let path = std::env::temp_dir().join(format!("agora-at-{}-{name}", std::process::id()));
        let l = local_listen(&path).expect("listen");
        l.set_nonblocking(true).expect("nonblocking");
        (l, path)
    }

    #[test]
    fn without_a_client_it_waits_the_timeout_and_says_none() {
        if !LOCAL_SOCKETS_SUPPORTED {
            return;
        }
        let (l, _path) = listener("none");
        let start = Instant::now();
        assert!(l
            .accept_timeout(Duration::from_millis(60))
            .unwrap()
            .is_none());
        let waited = start.elapsed();
        // It waited (no spin), and not much longer than asked; generous for a loaded runner.
        assert!(
            waited >= Duration::from_millis(50),
            "returned after {waited:?}"
        );
        assert!(waited < Duration::from_secs(5), "returned after {waited:?}");
    }

    #[test]
    fn a_client_is_returned_as_soon_as_it_connects_not_when_the_timeout_ends() {
        // #958: the control socket's accept loop slept 100 ms between tries, so each new connection waited up to that long.
        if !LOCAL_SOCKETS_SUPPORTED {
            return;
        }
        let (l, path) = listener("soon");
        let (tx, rx) = std::sync::mpsc::channel::<Instant>();
        let server = std::thread::spawn(move || {
            for _ in 0..20 {
                let s = loop {
                    if let Some(s) = l.accept_timeout(Duration::from_secs(30)).unwrap() {
                        break s;
                    }
                };
                tx.send(Instant::now()).unwrap();
                drop(s);
            }
        });
        let mut slowest = Duration::ZERO;
        for _ in 0..20 {
            // Let the server reach its wait first, so the client arrives in the middle of it.
            std::thread::sleep(Duration::from_millis(5));
            let sent = Instant::now();
            let c = local_connect(&path).expect("connect");
            let got = rx.recv_timeout(Duration::from_secs(20)).expect("accepted");
            slowest = slowest.max(got.saturating_duration_since(sent));
            drop(c);
        }
        server.join().unwrap();
        // A 30 s wait that returns within a second of every connect returned because of it.
        assert!(
            slowest < Duration::from_secs(1),
            "slowest accept {slowest:?}"
        );
    }

    #[test]
    fn a_temporary_name_is_split_only_in_the_forms_write_private_makes() {
        assert_eq!(split_temporary("key.pem.tmp-12-3"), Some(("key.pem", 12)));
        assert_eq!(split_temporary("key.pem.tmp-12"), Some(("key.pem", 12)));
        for other in [
            "key.pem.tmp-",
            "key.pem.tmp-12x",
            "key.pem.tmp-12-",
            "key.pem.tmp--3",
            "key.pem.tmp-1-2-3",
            "key.pem",
        ] {
            assert_eq!(split_temporary(other), None, "{other}");
        }
    }

    #[test]
    fn terminate_rejects_pid_zero() {
        let err = terminate_process(0).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        #[cfg(unix)]
        assert!(!unix::alive(0));
    }

    #[test]
    fn write_private_concurrent_threads_do_not_race() {
        let dir = std::env::temp_dir().join(format!("agora-wp-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("shared.txt");
        std::thread::scope(|s| {
            let mut handles = Vec::new();
            for i in 0..4 {
                let p = target.clone();
                handles.push(s.spawn(move || {
                    for j in 0..20 {
                        write_private(&p, format!("{i}-{j}").as_bytes()).unwrap();
                    }
                }));
            }
            for h in handles {
                h.join().unwrap();
            }
        });
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod small_file_tests {
    use super::read_small_regular_file_no_follow as read;

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("agora-small-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_small_regular_file_is_read_and_a_longer_one_is_refused() {
        let d = dir("len");
        let f = d.join("f");
        std::fs::write(&f, b"../..\n").unwrap();
        assert_eq!(read(&f, 16).unwrap(), b"../..\n");
        assert_eq!(read(&f, 6).unwrap(), b"../..\n");
        assert!(read(&f, 5).is_err());
        assert!(read(&d, 4096).is_err(), "a folder");
        assert!(read(&d.join("missing"), 4096).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_link_is_not_followed() {
        let d = dir("link");
        let target = d.join("target");
        std::fs::write(&target, b"x").unwrap();
        if crate::symlink(&target, &d.join("link")).is_ok() {
            assert!(read(&d.join("link"), 4096).is_err());
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A sparse file uses no disk and is still refused without being read. Unix only: NTFS allocates the length unless the file is
    /// marked sparse.
    #[cfg(unix)]
    #[test]
    fn a_huge_sparse_file_is_refused_unread() {
        let d = dir("sparse");
        let f = d.join("f");
        std::fs::write(&f, b"x").unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&f)
            .unwrap()
            .set_len(1 << 36)
            .unwrap();
        assert!(read(&f, 4096).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_is_refused_without_waiting() {
        let d = dir("fifo");
        let p = d.join("p");
        let c = std::ffi::CString::new(p.to_str().unwrap()).unwrap();
        // SAFETY: a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        let q = p.clone();
        std::thread::spawn(move || tx.send(read(&q, 4096).is_err()).unwrap());
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)), Ok(true));
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod program_name_tests {
    use super::same_program_name;

    /// ADR-0094: Windows names are compared ignoring ASCII case, every other system's exactly.
    #[test]
    fn a_program_name_is_compared_as_the_system_compares_file_names() {
        assert!(same_program_name("claude", "claude"));
        assert!(!same_program_name("claude", "codex"));
        assert!(!same_program_name("claude-x", "claude"));
        assert_eq!(same_program_name("CLAUDE", "claude"), cfg!(windows));
        assert_eq!(same_program_name("Codex", "codex"), cfg!(windows));
    }
}
