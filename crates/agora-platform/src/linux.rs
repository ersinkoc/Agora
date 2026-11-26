//! Process inspection through `/proc`.

use std::fs;
use std::io;
use std::path::PathBuf;

pub(crate) fn stats(pid: u32) -> io::Result<crate::ProcessStats> {
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "unexpected /proc format");
    // statm: size resident shared ... (in pages)
    let statm = fs::read_to_string(format!("/proc/{pid}/statm"))?;
    let pages: u64 = statm
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .ok_or_else(bad)?;
    // SAFETY: sysconf has no preconditions.
    let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).unwrap_or(4096);
    let ticks_per_s = u64::try_from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) }).unwrap_or(100);
    // stat: after the last ')': state ppid pgrp session tty tpgid flags minflt cminflt majflt cmajflt utime stime
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let rest = &stat[stat.rfind(')').ok_or_else(bad)? + 1..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    let ticks: u64 = [11, 12]
        .iter()
        .map(|i| f.get(*i).and_then(|v| v.parse::<u64>().ok()))
        .sum::<Option<u64>>()
        .ok_or_else(bad)?;
    Ok(crate::ProcessStats {
        rss_bytes: pages * page,
        cpu_seconds: ticks as f64 / ticks_per_s.max(1) as f64,
    })
}

pub(crate) fn name(pid: u32) -> io::Result<String> {
    let comm = fs::read_to_string(format!("/proc/{pid}/comm"))?;
    Ok(comm.trim_end_matches('\n').to_string())
}

pub(crate) fn cwd(pid: u32) -> io::Result<PathBuf> {
    fs::read_link(format!("/proc/{pid}/cwd"))
}

/// Parent pid from `/proc/<pid>/stat`: the field after the last `)` (the name may contain parentheses).
fn ppid(pid: u32) -> Option<u32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

pub(crate) fn children(pid: u32) -> io::Result<Vec<u32>> {
    let mut out = Vec::new();
    for entry in fs::read_dir("/proc")? {
        let Ok(entry) = entry else { continue };
        let Some(child) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        if ppid(child) == Some(pid) {
            out.push(child);
        }
    }
    out.sort_unstable();
    Ok(out)
}

// ---- Landlock (ADR-0047): file-system confinement the kernel enforces on a process and everything it starts ----

const SYS_LANDLOCK_CREATE_RULESET: libc::c_long = 444;
const SYS_LANDLOCK_ADD_RULE: libc::c_long = 445;
const SYS_LANDLOCK_RESTRICT_SELF: libc::c_long = 446;
const CREATE_RULESET_VERSION: libc::c_ulong = 1;
const RULE_PATH_BENEATH: libc::c_int = 1;
/// The first version's rights: execute, write/read file, read dir, remove, make (char, dir, reg, sock, fifo, block, sym).
const ALL_FS: u64 = (1 << 13) - 1;
const READ_FILE_FS: u64 = 1 << 2;
const READ_FS: u64 = 1 | READ_FILE_FS | (1 << 3);
/// Moving or linking a file into another folder (ABI 2). Not handled, every such move is refused, even inside `write`.
const REFER_FS: u64 = 1 << 13;
/// `truncate(2)` and opening with `O_TRUNC` (ABI 3). Not handled, a file anywhere can be cut to nothing.
const TRUNCATE_FS: u64 = 1 << 14;
/// What applies to a file rather than a directory: execute, write, read and truncate.
const FILE_FS: u64 = 1 | (1 << 1) | (1 << 2) | TRUNCATE_FS;

/// `LANDLOCK_SCOPE_ABSTRACT_UNIX_SOCKET` (ABI 6): no connecting to an abstract unix socket made outside the domain.
const SCOPE_ABSTRACT_UNIX_SOCKET: u64 = 1;
/// The first Landlock version that has scopes.
const ABI_SCOPES: libc::c_long = 6;

/// The kernel's attribute: file rights, network rights (ABI 4, never handled here) and scopes (ABI 6). The kernel takes a shorter
/// struct from an older program, so on a kernel below ABI 6 only the first field's size is passed.
#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
    handled_access_net: u64,
    scoped: u64,
}

#[repr(C, packed)]
struct PathBeneath {
    allowed_access: u64,
    parent_fd: i32,
}

/// The kernel's Landlock version (ABI), or a value below 1 when Landlock is off.
fn abi_version() -> libc::c_long {
    // SAFETY: the version query takes a null attribute and size 0.
    unsafe {
        libc::syscall(
            SYS_LANDLOCK_CREATE_RULESET,
            std::ptr::null::<RulesetAttr>(),
            0usize,
            CREATE_RULESET_VERSION,
        )
    }
}

/// True when the running kernel has Landlock switched on.
pub(crate) fn sandbox_supported() -> bool {
    abi_version() >= 1
}

/// True when the kernel can refuse connecting to an abstract unix socket made outside a sandbox (Landlock ABI 6, Linux 6.12). There
/// is no Landlock right for pathname sockets in the kernel headers this was built against, so those cannot be refused here.
pub(crate) fn scopes_abstract_sockets() -> bool {
    abi_version() >= ABI_SCOPES
}

/// A ruleset made before the fork, so the child only has to apply it (two async-signal-safe calls).
pub(crate) struct Ruleset {
    fd: std::os::fd::OwnedFd,
    pub(crate) git_config: Option<PathBuf>,
}

/// `folder`: the folder a start holds (F1012). Entries at or below its path are opened through the held folder, never by the path, so
/// they name the folder the process starts in even when the path led elsewhere for a moment while the rules were made (F1022).
/// `spelled`: the caller's own spelling of that folder's path (the start's `cwd`), when it named the folder through a link (an alias
/// the hold resolved): entries at or below that spelling are the held folder's too (Codex review of F1022).
pub(crate) fn ruleset(
    sb: &crate::Sandbox,
    folder: Option<&crate::HeldFolder>,
    spelled: Option<&std::path::Path>,
) -> io::Result<Ruleset> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    let abi = abi_version();
    if abi < 1 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this kernel has no Landlock",
        ));
    }
    // The rights later versions added are handled where the kernel knows them, so `write` alone may truncate and move files.
    let mut all = ALL_FS;
    if abi >= 2 {
        all |= REFER_FS;
    }
    if abi >= 3 {
        all |= TRUNCATE_FS;
    }
    let scope = sb.confine_sockets && abi >= ABI_SCOPES;
    let attr = RulesetAttr {
        handled_access_fs: all,
        handled_access_net: 0,
        scoped: if scope { SCOPE_ABSTRACT_UNIX_SOCKET } else { 0 },
    };
    let attr_size = if abi >= ABI_SCOPES {
        std::mem::size_of::<RulesetAttr>()
    } else {
        std::mem::size_of::<u64>()
    };
    // SAFETY: a valid attribute pointer and its size; the result is a new descriptor (close-on-exec) or -1.
    let fd = unsafe {
        libc::syscall(
            SYS_LANDLOCK_CREATE_RULESET,
            &attr as *const RulesetAttr,
            attr_size,
            0 as libc::c_ulong,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the kernel returned a fresh descriptor that nothing else owns.
    let set = unsafe { OwnedFd::from_raw_fd(fd as i32) };
    let closed = closed_places(&sb.closed)?;
    let git_config = sb.git_config_file();
    let mut granted_git_config = None;
    for (paths, rights) in [
        (sb.read.as_slice(), READ_FS),
        (sb.write.as_slice(), all),
        (git_config.as_slice(), READ_FILE_FS),
    ] {
        for p in paths {
            let file_only = rights == READ_FILE_FS;
            let below = folder.and_then(|h| {
                // The most specific spelling: one that lies inside the held folder (a link there) is resolved by the hold, never
                // opened again below the folder.
                let rel = [Some(h.path.as_path()), spelled]
                    .into_iter()
                    .flatten()
                    .filter_map(|at| p.strip_prefix(at).ok())
                    .min_by_key(|rel| rel.components().count())?;
                Some((h, rel))
            });
            let opened = match below {
                _ if file_only => {
                    use std::os::unix::fs::OpenOptionsExt;
                    std::fs::OpenOptions::new()
                        .read(true)
                        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                        .open(p)
                }
                Some((held, rel)) => open_below(held, rel),
                None => std::fs::File::open(p),
            };
            // A path that does not exist on this machine confines nothing; skip it.
            let Ok(f) = opened else {
                continue;
            };
            let metadata = match f.metadata() {
                Ok(m) => m,
                Err(_) if file_only => continue,
                Err(e) => return Err(e),
            };
            // #1150: what is checked is the descriptor the rule is made from, wherever its path led when it was opened.
            {
                use std::os::unix::fs::MetadataExt;
                if closed.contains(&(metadata.dev(), metadata.ino())) {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        crate::SandboxRefusal::ProtectedPathReachable,
                    ));
                }
            }
            if file_only {
                use std::os::unix::fs::MetadataExt;
                if !metadata.is_file() || metadata.nlink() != 1 {
                    continue;
                }
            }
            let rights = if metadata.is_dir() {
                rights
            } else {
                rights & FILE_FS
            };
            let rule = PathBeneath {
                allowed_access: rights,
                parent_fd: f.as_raw_fd(),
            };
            // SAFETY: valid ruleset descriptor, rule type and a pointer to a live rule.
            let r = unsafe {
                libc::syscall(
                    SYS_LANDLOCK_ADD_RULE,
                    set.as_raw_fd(),
                    RULE_PATH_BENEATH,
                    &rule as *const PathBeneath,
                    0 as libc::c_uint,
                )
            };
            if r < 0 {
                if file_only {
                    continue;
                }
                return Err(io::Error::last_os_error());
            }
            if file_only {
                granted_git_config = Some(p.clone());
            }
        }
    }
    Ok(Ruleset {
        fd: set,
        git_config: granted_git_config,
    })
}

/// #1150: the places no grant may be, as (device, inode): each of `closed`, and every folder above it as it is spelled (a link on the
/// way included), each where it leads now; then every folder above where it really is and above its name's folder (a link in its
/// place), up to `/`. A path that does not exist yet counts from its nearest existing folder. The node checked the grants by name
/// before the start; [`ruleset`] compares the descriptor each rule is made from with these, so a grant swapped for a link in between
/// (into the node's folder) refuses the start instead of granting where the link leads. Folders are climbed with `..` from an opened
/// folder; only a file's own folder is opened again, by the path its descriptor names (`/proc/self/fd`). Anything but a missing
/// component (no access, a loop of links, a file on the way, a climb that does not end) refuses the start: it fails closed.
fn closed_places(closed: &[PathBuf]) -> io::Result<Vec<(u64, u64)>> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let unresolved = || {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            crate::SandboxRefusal::ProtectedPathUnresolved,
        )
    };
    // `None`: the path does not exist (only that); any other failure is an error.
    let open = |p: &std::path::Path| -> io::Result<Option<std::fs::File>> {
        match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_CLOEXEC)
            .open(p)
        {
            Ok(f) => Ok(Some(f)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(unresolved()),
        }
    };
    let id = |f: &std::fs::File| -> io::Result<(u64, u64, bool)> {
        let m = f.metadata().map_err(|_| unresolved())?;
        Ok((m.dev(), m.ino(), m.is_dir()))
    };
    let mut out: Vec<(u64, u64)> = Vec::new();
    let record = |out: &mut Vec<(u64, u64)>, place: (u64, u64)| {
        if !out.contains(&place) {
            out.push(place);
        }
    };
    for p in closed {
        // A relative path (a `--data` given so) is taken from the node's folder, as everything else the node opens by it.
        let p = std::path::absolute(p).map_err(|_| unresolved())?;
        // By name: the path and every folder above it as spelled (the node's check compares names too).
        for a in p.ancestors() {
            if let Some(f) = open(a)? {
                let (dev, ino, _) = id(&f)?;
                record(&mut out, (dev, ino));
            }
        }
        // Where it leads: from the path, and from its name's folder, climbed to `/`.
        for start in std::iter::once(p.as_path()).chain(p.parent()) {
            let mut found = None;
            for a in start.ancestors() {
                if let Some(f) = open(a)? {
                    found = Some(f);
                    break;
                }
            }
            let mut at = found.ok_or_else(unresolved)?;
            let (dev, ino, dir) = id(&at)?;
            if !dir {
                // A file (the control token): itself, then the folder it really is in.
                record(&mut out, (dev, ino));
                let real = opened_path(&at).map_err(|_| unresolved())?;
                let parent = real.parent().ok_or_else(unresolved)?;
                at = open(parent)?.ok_or_else(unresolved)?;
            }
            // Bounded: a path of at most PATH_MAX bytes has fewer folders than that. A climb that does not reach `/` refuses.
            let mut top = false;
            for _ in 0..libc::PATH_MAX {
                let (dev, ino, _) = id(&at)?;
                record(&mut out, (dev, ino));
                let up = open_dotdot(&at).map_err(|_| unresolved())?;
                let (up_dev, up_ino, _) = id(&up)?;
                if (up_dev, up_ino) == (dev, ino) {
                    top = true;
                    break;
                }
                at = up;
            }
            if !top {
                return Err(unresolved());
            }
        }
    }
    Ok(out)
}

/// The folder above `dir` (`..` opened from its descriptor, as a place only).
fn open_dotdot(dir: &std::fs::File) -> io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    // SAFETY: a live directory descriptor and a NUL-terminated name; the result is a new descriptor or -1.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            c"..".as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the kernel returned a fresh descriptor that nothing else owns.
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

/// F1022: `rel` (empty: the folder itself) opened from the held folder's handle, as a place only (`O_PATH`, which Landlock takes and
/// which needs no read permission on it).
fn open_below(held: &crate::HeldFolder, rel: &std::path::Path) -> io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    if rel.as_os_str().is_empty() {
        return held.dir.try_clone();
    }
    let name = std::ffi::CString::new(rel.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: a live directory descriptor and a NUL-terminated relative name; the result is a new descriptor or -1.
    let fd = unsafe {
        libc::openat(
            held.dir.as_raw_fd(),
            name.as_ptr(),
            libc::O_PATH | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the kernel returned a fresh descriptor that nothing else owns.
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

/// See [`crate::HeldFolder::open`]: the folder opened as a place only (`O_PATH|O_DIRECTORY`). That needs search permission on the
/// folders above it and none on the folder itself, so a folder the user may enter but not list can be held too (Codex review of
/// F1021); everything a held folder is used for works on it: `fchdir`, `/proc/self/fd`, `openat` below it, and Landlock rules.
pub(crate) fn open_held_dir(path: &std::path::Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_DIRECTORY)
        .open(path)
}

impl Ruleset {
    /// Confines the calling process for good. Only async-signal-safe calls, so it may run between fork and exec.
    pub(crate) fn enforce(&self) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        // SAFETY: plain prctl and the restrict call on a descriptor this value owns.
        unsafe {
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::syscall(
                SYS_LANDLOCK_RESTRICT_SELF,
                self.fd.as_raw_fd(),
                0 as libc::c_uint,
            ) != 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
}
/// See [`crate::opened_path`]: the descriptor's link in `/proc/self/fd`.
pub(crate) fn opened_path(file: &std::fs::File) -> io::Result<PathBuf> {
    use std::os::fd::AsRawFd;
    fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

/// Process group and start time (clock ticks after boot, `/proc/<pid>/stat` field 22) of a running process.
pub(crate) fn identity(pid: u32) -> io::Result<crate::ProcessIdentity> {
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "unexpected /proc format");
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    // After the last ')': state(0) ppid(1) pgrp(2) ... starttime(19).
    let rest = &stat[stat.rfind(')').ok_or_else(bad)? + 1..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    if f.first() == Some(&"Z") {
        return Err(io::Error::new(io::ErrorKind::NotFound, "zombie"));
    }
    let group = f.get(2).and_then(|v| v.parse().ok()).ok_or_else(bad)?;
    let started = f.get(19).and_then(|v| v.parse().ok()).ok_or_else(bad)?;
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        .trim()
        .to_string();
    Ok(crate::ProcessIdentity {
        pid,
        group,
        started,
        boot,
        name: name(pid)?,
    })
}

/// See [`crate::process_start_time`]: field 22 of `/proc/<pid>/stat` (clock ticks after boot); a zombie counts as ended.
pub(crate) fn start_time(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &stat[stat.rfind(')')? + 1..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    if f.first() == Some(&"Z") || f.first() == Some(&"X") {
        return None;
    }
    // Fields after the name start at field 3 (state), so field 22 is index 19.
    f.get(19)?.parse().ok()
}

/// See [`crate::end_with_supervisor`]: `SIGTERM` when the supervisor ends (`PR_SET_PDEATHSIG`), and at once when it ended before the
/// watch was set (this process was handed to another parent already).
pub(crate) fn end_with(supervisor: u32) {
    // SAFETY: plain system calls with no pointers.
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
        if u32::try_from(libc::getppid()).ok() != Some(supervisor) {
            libc::raise(libc::SIGTERM);
        }
    }
}

// `agora service` on Linux (ADR-0062 item 6): a systemd user unit, for this user only and never with root, sudo or `loginctl`.
//
// - The unit runs the supervisor in the foreground (`Type=simple`); `KillMode=control-group` ends everything the role started when the
//   unit stops, after `TimeoutStopSec=25`; the node's unit has `KillMode=process` (see `unit_text`) so its holder and agents stay. `Restart=on-failure` (when the spec asks for a restart) starts again
//   a supervisor that crashed, was killed by a signal (an OOM kill) or timed out. Under a service (`--log`) its planned ends (give-up,
//   refusal, lost lock, clean stop) exit 0 and are never restarted (decision `2026-11-18-1100-linux-563`). Nothing else restarts.
// - The unit file is written owner-only (0600) and atomically into `$XDG_CONFIG_HOME/systemd/user` (`~/.config/systemd/user`),
//   never through a link at its own place. `ExecStart` is quoted for systemd itself (no shell): every word in double quotes, `\` and
//   `"` escaped, `%` written `%%` (a specifier) and `$` written `$$` (a variable). Only `PATH` goes into `Environment=`.
// - `systemctl` is run with `LC_ALL=C`, so its answers can be read; the reason of a failure is passed on, never parsed further.

/// The first line of every unit Agora writes. A unit at the same place without it (written by hand, by another tool) is never
/// replaced, disabled or deleted: [`crate::ServiceProblem::NotOurs`].
const UNIT_MARKER: &str = "# Login service (ADR-0062 item 6): written and removed by its own service command; edits are not kept.\n";

/// See [`crate::desktop_notify`]: `notify-send` (libnotify), found in a full-path folder of `PATH` and run by that path.
pub(crate) fn desktop_notify(title: &str, body: &str) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let program = std::env::var_os("PATH")
        .and_then(|p| {
            std::env::split_paths(&p)
                .filter(|d| d.is_absolute())
                .map(|d| d.join("notify-send"))
                .find(|p| {
                    fs::metadata(p)
                        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                })
        })
        .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    let status = std::process::Command::new(program)
        // The application name is the title the caller gave (from `locales/`), never a product name written here.
        .arg(format!("--app-name={title}"))
        .args(["--", title, body])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("notify-send: {status}")))
    }
}

/// See [`crate::services_available`]: an executable `systemctl` in a full-path folder of `PATH`.
pub(crate) fn services_available() -> bool {
    std::env::var_os("PATH").is_some_and(|p| systemctl_in(&p).is_some())
}

/// The first executable `systemctl` in the full-path folders of `path`, by its full path. Empty and relative entries are skipped, so
/// a `systemctl` planted in whatever folder Agora runs from is never run; every call uses this path, never the bare name.
fn systemctl_in(path: &std::ffi::OsStr) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(path)
        .filter(|d| d.is_absolute())
        .map(|d| d.join("systemctl"))
        .find(|p| fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}

/// The systemctl to run and the folder of user units; tests give a fake systemctl and a scratch folder.
struct Systemd {
    systemctl: PathBuf,
    unit_dir: PathBuf,
}

impl Systemd {
    /// `systemctl` from `PATH`, and the user unit folder systemd reads first: `$XDG_CONFIG_HOME/systemd/user` when that is a full
    /// path (the XDG rule), otherwise `$HOME/.config/systemd/user`.
    fn from_env() -> io::Result<Self> {
        // SAFETY: plain call.
        let euid = unsafe { libc::geteuid() };
        let unit_dir = unit_dir_for(
            euid,
            std::env::var_os("XDG_CONFIG_HOME"),
            std::env::var_os("HOME"),
        )?;
        let systemctl = std::env::var_os("PATH")
            .and_then(|p| systemctl_in(&p))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "systemctl"))?;
        Ok(Systemd {
            systemctl,
            unit_dir,
        })
    }

    /// `systemctl --user show` of the unit: the file systemd loaded it from (empty when it found none) and its drop-ins.
    fn loaded_from(&self, name: &str) -> io::Result<(Option<PathBuf>, Vec<PathBuf>)> {
        let out = self.run(&[
            "show",
            "-p",
            "FragmentPath",
            "-p",
            "DropInPaths",
            &unit_name(name),
        ])?;
        if !out.status.success() {
            return Err(failure(&out));
        }
        Ok(read_show(&String::from_utf8_lossy(&out.stdout)))
    }

    /// Whether systemd runs exactly the unit at `path`: loaded from that file, with no drop-in (`<name>.service.d/`, `agora-.service.d/`,
    /// `service.d/`, `/etc/systemd/user`, …) that could change what it runs. `Err` names what else it reads.
    fn runs_only(&self, name: &str, path: &std::path::Path) -> io::Result<()> {
        let (fragment, drop_ins) = self.loaded_from(name)?;
        let Some(fragment) = fragment else {
            return Err(io::Error::other(crate::ServiceProblem::NotLoaded(
                path.to_path_buf(),
            )));
        };
        let same = |a: &std::path::Path, b: &std::path::Path| {
            a == b
                || fs::canonicalize(a)
                    .ok()
                    .is_some_and(|a| fs::canonicalize(b).ok() == Some(a))
        };
        let mut others = drop_ins;
        if !same(&fragment, path) {
            others.insert(0, fragment);
        }
        if others.is_empty() {
            Ok(())
        } else {
            Err(io::Error::other(crate::ServiceProblem::Overridden(others)))
        }
    }

    /// See [`crate::service_drop_ins`].
    fn drop_ins(&self, name: &str) -> io::Result<Vec<PathBuf>> {
        check_name(name)?;
        let (fragment, mut others) = self.loaded_from(name)?;
        let path = self.unit_path(name);
        if let Some(f) = fragment {
            if f != path && fs::canonicalize(&f).ok() != fs::canonicalize(&path).ok() {
                others.insert(0, f);
            }
        }
        Ok(others)
    }

    /// See [`crate::service_unit_differs`]: the text Agora would write now is not the text in its unit file.
    fn differs(&self, spec: &crate::ServiceSpec) -> io::Result<bool> {
        use std::io::Read;
        use std::os::unix::fs::OpenOptionsExt;
        check_name(&spec.name)?;
        let mut spec = spec.clone();
        if let Ok(real) = fs::canonicalize(&spec.work_dir) {
            spec.work_dir = real;
        }
        let text = unit_text(&spec)?;
        let path = self.unit_path(&spec.name);
        let mut f = match fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(true),
            Err(e) => return Err(e),
        };
        let mut was = Vec::new();
        f.read_to_end(&mut was)?;
        Ok(was != text.as_bytes())
    }

    fn unit_path(&self, name: &str) -> PathBuf {
        self.unit_dir.join(unit_name(name))
    }

    fn run(&self, args: &[&str]) -> io::Result<std::process::Output> {
        std::process::Command::new(&self.systemctl)
            .arg("--user")
            .args(args)
            .env("LC_ALL", "C")
            .env("SYSTEMD_PAGER", "")
            .stdin(std::process::Stdio::null())
            .output()
    }

    /// Runs `systemctl --user <args>`; its own words become the error when it fails.
    fn checked(&self, args: &[&str]) -> io::Result<()> {
        let out = self.run(args)?;
        if out.status.success() {
            return Ok(());
        }
        Err(failure(&out))
    }

    fn status(&self, name: &str) -> io::Result<crate::ServiceState> {
        check_name(name)?;
        let out = self.run(&["is-enabled", &unit_name(name)])?;
        read_is_enabled(&out)
    }

    fn install(&self, spec: &crate::ServiceSpec) -> io::Result<()> {
        check_name(&spec.name)?;
        // systemd reads `WorkingDirectory=` only in its plain form (no `..`): the folder's real path, which is the same folder.
        let mut spec = spec.clone();
        if let Ok(real) = fs::canonicalize(&spec.work_dir) {
            spec.work_dir = real;
        }
        let text = unit_text(&spec)?;
        let path = self.unit_path(&spec.name);
        let before = write_unit(&self.unit_dir, &path, &text)?;
        // Enabled only once systemd is seen to run exactly this file: a drop-in or another unit file of the same name would change
        // what it runs, and is named instead.
        let registered = self
            .checked(&["daemon-reload"])
            .and_then(|()| self.runs_only(&spec.name, &path))
            .and_then(|()| self.checked(&["enable", &unit_name(&spec.name)]));
        if registered.is_err() {
            // Leave nothing half-registered behind: what was there before is put back (nothing, or Agora's earlier unit).
            match &before {
                Before::Absent => {
                    let _ = fs::remove_file(&path);
                }
                Before::Other(was) => {
                    let _ = crate::write_private(&path, was);
                }
                Before::Same => {}
            }
            if !matches!(before, Before::Same) {
                let _ = self.checked(&["daemon-reload"]);
            }
        }
        registered
    }

    /// See [`crate::service_foreign`]: the unit is registered but its file in the unit folder is not one Agora wrote (no marker, a
    /// link, not a plain file), or it is registered from another folder.
    fn foreign(&self, name: &str) -> io::Result<Option<PathBuf>> {
        let state = self.status(name)?;
        Ok(self.foreign_of(state, name))
    }

    fn foreign_of(&self, state: crate::ServiceState, name: &str) -> Option<PathBuf> {
        if state == crate::ServiceState::NotInstalled {
            return None;
        }
        let path = self.unit_path(name);
        match ours(&path) {
            Ok(true) => None,
            Ok(false) => Some(path),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Some(PathBuf::from(unit_name(name))),
            Err(_) => Some(path),
        }
    }

    fn start(&self, name: &str) -> io::Result<()> {
        check_name(name)?;
        self.checked(&["start", &unit_name(name)])
    }

    fn remove(&self, name: &str) -> io::Result<bool> {
        check_name(name)?;
        let state = self.status(name)?;
        let path = self.unit_path(name);
        let file = fs::symlink_metadata(&path).ok();
        if state == crate::ServiceState::NotInstalled && file.is_none() {
            return Ok(false);
        }
        // Only a unit Agora wrote is turned off and deleted; one written by hand under the same name is left exactly as it is.
        if let Some(theirs) = self.foreign_of(state, name) {
            return Err(not_ours(theirs));
        }
        if file.is_some() && !ours(&path).unwrap_or(false) {
            return Err(not_ours(path));
        }
        if state != crate::ServiceState::NotInstalled {
            // Stops it too, if it still runs (the caller stopped the role through its lock already).
            let out = self.run(&["disable", "--now", &unit_name(name)])?;
            if !out.status.success() && !not_found(&out) {
                return Err(failure(&out));
            }
        }
        match fs::symlink_metadata(&path) {
            Ok(m) if m.is_file() => fs::remove_file(&path)?,
            // A link or a folder there was not written by Agora: left alone and named.
            Ok(_) => return Err(in_the_way(path)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        self.checked(&["daemon-reload"])?;
        Ok(true)
    }
}

/// What was at the unit's place before [`write_unit`].
enum Before {
    Absent,
    /// The same text: nothing was written.
    Same,
    /// An earlier unit Agora wrote, with other text.
    Other(Vec<u8>),
}

/// The user unit folder systemd reads first: `$XDG_CONFIG_HOME/systemd/user` when that is a full path (the XDG rule), otherwise
/// `$HOME/.config/systemd/user`. Refused for root ([`crate::ServiceProblem::Root`]): `sudo` would register root's own service, or
/// leave root's files in the user's folder.
fn unit_dir_for(
    euid: u32,
    xdg: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> io::Result<PathBuf> {
    if euid == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            crate::ServiceProblem::Root,
        ));
    }
    xdg.map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            home.map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|h| h.join(".config"))
        })
        .map(|c| c.join("systemd").join("user"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                crate::ServiceProblem::NoConfigFolder,
            )
        })
}

/// `FragmentPath=` and `DropInPaths=` of `systemctl show` (the drop-ins separated by spaces).
fn read_show(text: &str) -> (Option<PathBuf>, Vec<PathBuf>) {
    let mut fragment = None;
    let mut drop_ins = Vec::new();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("FragmentPath=") {
            fragment = (!v.trim().is_empty()).then(|| PathBuf::from(v.trim()));
        } else if let Some(v) = line.strip_prefix("DropInPaths=") {
            drop_ins = split_drop_ins(v);
        }
    }
    (fragment, drop_ins)
}

/// `DropInPaths=` separates its paths by spaces, and a path may hold one: every drop-in ends in `.conf`, so words are joined until one
/// does (a last word without it is kept as it is).
fn split_drop_ins(v: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in v.split(' ') {
        if word.is_empty() && cur.is_empty() {
            continue;
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
        if word.ends_with(".conf") {
            out.push(PathBuf::from(std::mem::take(&mut cur)));
        }
    }
    let rest = cur.trim();
    if !rest.is_empty() {
        out.push(PathBuf::from(rest));
    }
    out
}

pub(crate) fn service_precheck() -> io::Result<()> {
    Systemd::from_env().map(|_| ())
}

pub(crate) fn service_unit_differs(spec: &crate::ServiceSpec) -> io::Result<bool> {
    Systemd::from_env()?.differs(spec)
}

pub(crate) fn service_drop_ins(name: &str) -> io::Result<Vec<PathBuf>> {
    Systemd::from_env()?.drop_ins(name)
}

fn unit_name(name: &str) -> String {
    format!("{name}.service")
}

/// A unit name Agora gives (`agora-node`): letters, digits and `-_.:` only, so it is one plain word for systemctl and the file system.
fn check_name(name: &str) -> io::Result<()> {
    if name.is_empty()
        || name.starts_with('.')
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
    {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    Ok(())
}

fn not_ours(path: PathBuf) -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        crate::ServiceProblem::NotOurs(path),
    )
}

/// Whether the unit file at `path` is one Agora wrote: a plain file (opened without following a link) that starts with the marker.
fn ours(path: &std::path::Path) -> io::Result<bool> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    if !fs::symlink_metadata(path)?.is_file() {
        return Ok(false);
    }
    let f = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let mut head = Vec::with_capacity(UNIT_MARKER.len());
    f.take(UNIT_MARKER.len() as u64).read_to_end(&mut head)?;
    Ok(head == UNIT_MARKER.as_bytes())
}

fn in_the_way(path: PathBuf) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        crate::ServiceProblem::InTheWay(path),
    )
}

/// systemctl's message when it could not reach the user's service manager (with `LC_ALL=C`).
fn no_user_manager(said: &str) -> bool {
    [
        "Failed to connect to bus",
        "Failed to connect to user scope bus",
        "Failed to get D-Bus connection",
        "not been booted with systemd",
        "XDG_RUNTIME_DIR",
    ]
    .iter()
    .any(|m| said.contains(m))
}

/// systemctl's message for a unit it does not know (with `LC_ALL=C`; newer versions print `not-found` instead).
fn not_found(out: &std::process::Output) -> bool {
    let said = String::from_utf8_lossy(&out.stderr);
    said.contains("No such file or directory")
        || said.contains("does not exist")
        || said.contains("not found")
        || String::from_utf8_lossy(&out.stdout).trim() == "not-found"
}

fn failure(out: &std::process::Output) -> io::Error {
    let said = String::from_utf8_lossy(if out.stderr.is_empty() {
        &out.stdout
    } else {
        &out.stderr
    })
    .trim()
    .to_string();
    if no_user_manager(&said) {
        return io::Error::new(
            io::ErrorKind::NotConnected,
            crate::ServiceProblem::NoUserManager,
        );
    }
    if said.is_empty() {
        return io::Error::other(format!("systemctl: {}", out.status));
    }
    io::Error::other(said)
}

/// `systemctl --user is-enabled <unit>`: its word on standard output, or "not found" from a failure; any other failure (no bus, a
/// broken manager) is an error, never "not installed".
fn read_is_enabled(out: &std::process::Output) -> io::Result<crate::ServiceState> {
    use crate::ServiceState::{Installed, NotInstalled};
    let said = String::from_utf8_lossy(&out.stderr);
    if no_user_manager(&said) {
        return Err(failure(out));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    match stdout.lines().next().map(str::trim).unwrap_or("") {
        // Only an enabled unit starts at login.
        "enabled" | "enabled-runtime" => Ok(Installed { enabled: true }),
        // Present, but it does not start at login by its own `[Install]`: turned off (`disable`, `mask`), linked in by hand, or a
        // unit Agora would not have made (static, indirect, generated, an alias, transient).
        "disabled" | "linked" | "linked-runtime" | "masked" | "masked-runtime" | "bad"
        | "static" | "indirect" | "generated" | "alias" | "transient" => {
            Ok(Installed { enabled: false })
        }
        "not-found" => Ok(NotInstalled),
        "" if !out.status.success() && not_found(out) => Ok(NotInstalled),
        _ => Err(failure(out)),
    }
}

/// See [`crate::service_refusal`]: a systemd unit is UTF-8 and read line by line; the program and the folder are full paths, and the
/// folder has no space at either end (systemd would drop it).
pub(crate) fn service_refusal(
    spec: &crate::ServiceSpec,
) -> Option<(std::ffi::OsString, crate::ServiceRefusal)> {
    use crate::ServiceRefusal::{CannotCarry, ControlCharacter, NotAbsolute};
    fn text(s: &std::ffi::OsStr) -> Result<&str, crate::ServiceRefusal> {
        match s.to_str() {
            None => Err(CannotCarry),
            Some(t) if t.chars().any(char::is_control) => Err(ControlCharacter),
            Some(t) => Ok(t),
        }
    }
    let path = |p: &std::path::Path, folder: bool| {
        let t = text(p.as_os_str())?;
        if !p.is_absolute() {
            return Err(NotAbsolute);
        }
        // systemd refuses an executable whose path holds a quote or a backslash ("Executable path contains special characters",
        // measured with `systemd-analyze verify`, systemd 259).
        if !folder && t.contains(['"', '\'', '\\']) {
            return Err(crate::ServiceRefusal::ProgramCharacter);
        }
        if folder && t != t.trim() {
            return Err(CannotCarry);
        }
        // `WorkingDirectory=` is not quoted: a `\` at a line's end joins the next line, and no other use of it is relied on.
        if folder && t.contains('\\') {
            return Err(crate::ServiceRefusal::Backslash);
        }
        Ok(())
    };
    let named = |s: &std::ffi::OsStr, why| Some((s.to_owned(), why));
    if let Err(why) = path(&spec.program, false) {
        return named(spec.program.as_os_str(), why);
    }
    for a in &spec.args {
        if let Err(why) = text(a) {
            return named(a, why);
        }
    }
    if let Err(why) = path(&spec.work_dir, true) {
        return named(spec.work_dir.as_os_str(), why);
    }
    None
}

/// The executable, the first word of `ExecStart=`: in double quotes with `%` as `%%`. systemd expands no variable there and does not
/// undo `$$` (measured), so `$` stays as it is; quotes and backslashes are refused before ([`service_refusal`]).
fn exec_program(s: &str) -> String {
    format!("\"{}\"", s.replace('%', "%%"))
}

/// An argument of `ExecStart=`: in double quotes, with `\` and `"` escaped (systemd unescapes C style), `%` as `%%` (a specifier) and
/// `$` as `$$` (a variable). No shell reads it.
fn exec_word(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%' => out.push_str("%%"),
            '$' => out.push_str("$$"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A value of `Environment=` (one quoted assignment): `\` and `"` escaped, `%` as `%%`. systemd does not expand `$` there.
fn env_word(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%' => out.push_str("%%"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The unit file for `spec` (see the notes above); `InvalidInput` for what [`service_refusal`] refuses.
fn unit_text(spec: &crate::ServiceSpec) -> io::Result<String> {
    let invalid = || io::Error::from(io::ErrorKind::InvalidInput);
    if service_refusal(spec).is_some() {
        return Err(invalid());
    }
    let work_dir = spec.work_dir.to_str().ok_or_else(invalid)?;
    if spec
        .work_dir
        .components()
        .any(|c| c == std::path::Component::ParentDir)
    {
        return Err(invalid());
    }
    let mut exec = exec_program(spec.program.to_str().ok_or_else(invalid)?);
    for a in &spec.args {
        exec.push(' ');
        exec.push_str(&exec_word(a.to_str().ok_or_else(invalid)?));
    }
    let description: String = spec
        .description
        .chars()
        .map(|c| if c.is_control() || c == '\\' { ' ' } else { c })
        .collect::<String>()
        .trim()
        .replace('%', "%%");
    let mut unit = format!(
        "{UNIT_MARKER}[Unit]\nDescription={description}\n\n[Service]\nType=simple\nExecStart={exec}\nWorkingDirectory={}\n",
        work_dir.replace('%', "%%")
    );
    if let Some(path) = crate::service_path_variable(&spec.env) {
        unit.push_str(&format!(
            "Environment={}\n",
            env_word(&format!("PATH={path}"))
        ));
    }
    // `control-group` (systemd's default) signals every process in the unit's cgroup when it stops, which would end the terminal holder
    // and the agents it keeps (ADR-0079). `process` signals only the main process, the supervisor. That is enough: the supervisor
    // catches `SIGTERM` and passes it on to the node (the path `agora down` uses); a supervisor killed outright (`SIGKILL` after
    // `TimeoutStopSec`) still ends the node, which sets `PR_SET_PDEATHSIG` (`end_with`). The holder and the agents are each in a
    // session of their own (`setsid`) and are not the supervisor's children, so nothing signals them and they keep running. The next
    // start is a new supervisor and node, which attach to the holder that is still there. systemd logs "left-over process" for them:
    // expected. The hub keeps `control-group`: it has no such children.
    let kill_mode = if spec.keep_children_on_stop {
        "process"
    } else {
        "control-group"
    };
    unit.push_str(&format!("KillMode={kill_mode}\nTimeoutStopSec=25\n"));
    if spec.restart_supervisor_on_failure {
        unit.push_str("Restart=on-failure\n");
    }
    unit.push_str("\n[Install]\nWantedBy=default.target\n");
    Ok(unit)
}

/// Writes the unit at `path` in `dir` (made 0700 when missing), owner-only and atomically, and says what was there. A link, anything
/// but a plain file, or a unit Agora did not write at `path` is refused, never followed or replaced.
fn write_unit(dir: &std::path::Path, path: &std::path::Path, text: &str) -> io::Result<Before> {
    use std::io::Read;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    match fs::symlink_metadata(path) {
        Ok(m) if !m.is_file() => return Err(in_the_way(path.to_path_buf())),
        Ok(_) => {
            let mut f = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path)
                .map_err(|e| match e.raw_os_error() {
                    Some(libc::ELOOP) => in_the_way(path.to_path_buf()),
                    _ => e,
                })?;
            let mut was = Vec::new();
            f.read_to_end(&mut was)?;
            if was == text.as_bytes() {
                return Ok(Before::Same);
            }
            // A unit Agora did not write is never replaced.
            if !was.starts_with(UNIT_MARKER.as_bytes()) {
                return Err(not_ours(path.to_path_buf()));
            }
            crate::write_private(path, text.as_bytes())?;
            return Ok(Before::Other(was));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    // Written beside it (exclusive, 0600 from its creation) and renamed over it: `rename` replaces the name, never a link's target.
    crate::write_private(path, text.as_bytes())?;
    Ok(Before::Absent)
}

pub(crate) fn service_install(spec: &crate::ServiceSpec) -> io::Result<()> {
    Systemd::from_env()?.install(spec)
}

pub(crate) fn service_foreign(name: &str) -> io::Result<Option<PathBuf>> {
    Systemd::from_env()?.foreign(name)
}

pub(crate) fn service_start(name: &str) -> io::Result<()> {
    Systemd::from_env()?.start(name)
}

pub(crate) fn service_remove(name: &str) -> io::Result<bool> {
    Systemd::from_env()?.remove(name)
}

pub(crate) fn service_status(name: &str) -> io::Result<crate::ServiceState> {
    Systemd::from_env()?.status(name)
}

/// [`crate::rename_no_replace`] on Linux: `renameat2(RENAME_NOREPLACE)`, made as a system call (a C library older than glibc 2.28
/// has no wrapper). `ENOSYS` (a kernel before 3.15) and `EINVAL` (a file system without the flag) are `Unsupported`.
pub(crate) fn rename_no_replace(from: &std::path::Path, to: &std::path::Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = |p: &std::path::Path| {
        std::ffi::CString::new(p.as_os_str().as_bytes())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
    };
    let (from, to) = (c(from)?, c(to)?);
    // SAFETY: two NUL-terminated paths that outlive the call; relative ones resolve against the working folder (`AT_FDCWD`).
    let r = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if r == 0 {
        return Ok(());
    }
    let e = io::Error::last_os_error();
    match e.raw_os_error() {
        Some(libc::ENOSYS) | Some(libc::EINVAL) => Err(io::Error::from(io::ErrorKind::Unsupported)),
        _ => Err(e),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod service_tests {
    use super::*;
    use crate::{ServiceProblem, ServiceRefusal, ServiceSpec, ServiceState};
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::Path;

    /// A fake `systemctl` that records its arguments and answers from files beside it: `units/` is the unit folder, `enabled-<unit>`
    /// marks an enabled unit, `mode` holds `bus` (no user bus), `broken` (another failure), `new` (systemd's `not-found` word) or
    /// `failenable`. The real systemctl is never run.
    const FAKE: &str = r#"#!/bin/sh
d=$(dirname "$0")
printf '%s\n' "$*" >> "$d/log"
mode=$(cat "$d/mode" 2>/dev/null)
case "$mode" in
  bus) echo "Failed to connect to bus: No medium found" >&2; exit 1;;
  broken) echo "Access denied" >&2; exit 1;;
esac
[ "$1" = --user ] || { echo "not --user" >&2; exit 99; }
shift
case "$1" in
  probe) exit 0;;
  is-enabled)
    if [ -f "$d/units/$2" ]; then
      if [ -e "$d/enabled-$2" ]; then echo enabled; exit 0; fi
      echo disabled; exit 1
    fi
    if [ "$mode" = new ]; then echo not-found; exit 4; fi
    echo "Failed to get unit file state for $2: No such file or directory" >&2; exit 1;;
  enable)
    if [ "$mode" = failenable ]; then echo "Failed to enable unit: boom" >&2; exit 1; fi
    touch "$d/enabled-$2";;
  disable) [ "$2" = --now ] && shift; rm -f "$d/enabled-$2";;
  show)
    u="$6"; f=""
    [ -f "$d/units/$u" ] && f="$d/units/$u"
    [ "$mode" = elsewhere ] && f=""
    echo "FragmentPath=$f"
    di=""
    for x in "$d/units/$u.d/"*.conf "$d/units/agora-.service.d/"*.conf; do [ -e "$x" ] && di="$di $x"; done
    echo "DropInPaths=${di# }";;
  daemon-reload|start) ;;
  *) exit 98;;
esac
"#;

    struct Fake {
        dir: PathBuf,
        systemd: Systemd,
    }

    impl Fake {
        fn new(tag: &str) -> Fake {
            let dir =
                std::env::temp_dir().join(format!("agora-systemd-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let script = dir.join("systemctl");
            fs::write(&script, FAKE).unwrap();
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
            let systemd = Systemd {
                systemctl: script,
                unit_dir: dir.join("units"),
            };
            // A test thread forking at the moment the script was written can hold it open for a moment (ETXTBSY): wait it out.
            for _ in 0..100 {
                if systemd.run(&["probe"]).is_ok() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            fs::write(dir.join("log"), "").unwrap();
            Fake { dir, systemd }
        }
        fn mode(&self, m: &str) {
            fs::write(self.dir.join("mode"), m).unwrap();
        }
        fn log(&self) -> String {
            fs::read_to_string(self.dir.join("log")).unwrap()
        }
        fn unit(&self) -> PathBuf {
            self.dir.join("units").join("agora-node.service")
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn spec(home: &Path) -> ServiceSpec {
        ServiceSpec {
            name: "agora-node".into(),
            description: "Starts the Agora node when you log in (it keeps it running).".into(),
            program: PathBuf::from("/opt/agora/bin/agora"),
            args: vec![
                "supervise".into(),
                "node".into(),
                "--home".into(),
                home.as_os_str().to_owned(),
                "--log".into(),
            ],
            env: vec![("PATH".into(), "/usr/local/bin:/usr/bin".into())],
            work_dir: home.to_path_buf(),
            restart_supervisor_on_failure: true,
            keep_children_on_stop: false,
        }
    }

    #[test]
    fn the_unit_runs_the_supervisor_in_the_foreground_and_restarts_it_only_after_a_failure() {
        let text = unit_text(&spec(Path::new("/home/u/.agora"))).unwrap();
        assert_eq!(
            text,
            "# Login service (ADR-0062 item 6): written and removed by its own service command; edits are not kept.\n\
             [Unit]\nDescription=Starts the Agora node when you log in (it keeps it running).\n\n[Service]\nType=simple\n\
             ExecStart=\"/opt/agora/bin/agora\" \"supervise\" \"node\" \"--home\" \"/home/u/.agora\" \"--log\"\n\
             WorkingDirectory=/home/u/.agora\nEnvironment=\"PATH=/usr/local/bin:/usr/bin\"\nKillMode=control-group\n\
             TimeoutStopSec=25\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n"
        );
        let mut s = spec(Path::new("/home/u/.agora"));
        s.restart_supervisor_on_failure = false;
        assert!(!unit_text(&s).unwrap().contains("Restart="));
    }

    #[test]
    fn only_a_unit_that_keeps_its_children_uses_kill_mode_process() {
        let mut s = spec(Path::new("/home/u/.agora"));
        let hub = unit_text(&s).unwrap();
        assert!(hub.contains("KillMode=control-group\nTimeoutStopSec=25\n"));
        assert!(!hub.contains("KillMode=process"));
        s.keep_children_on_stop = true;
        let node = unit_text(&s).unwrap();
        assert!(node.contains("KillMode=process\nTimeoutStopSec=25\n"));
        assert!(!node.contains("control-group"));
        // Only that line differs: the quoting and the rest are the same.
        assert_eq!(
            node.replace("KillMode=process", "KillMode=control-group"),
            hub
        );
    }

    #[test]
    fn tricky_paths_are_quoted_for_systemd_without_a_shell() {
        let home = Path::new("/home/u/my \"q\" 'x' 50% $HOME ${X} ö ş/");
        let mut s = spec(home);
        // A backslash is refused in the folder, but an argument carries it escaped inside its quotes.
        s.args[3] = "/home/u/my \"q\" 'x' 50% $HOME ${X} ö\\ ş/".into();
        s.program = PathBuf::from("/opt/my apps/agora");
        s.description = "100%\nsure".into();
        let text = unit_text(&s).unwrap();
        assert!(
            text.contains(
                "ExecStart=\"/opt/my apps/agora\" \"supervise\" \"node\" \"--home\" \
                 \"/home/u/my \\\"q\\\" 'x' 50%% $$HOME $${X} ö\\\\ ş/\" \"--log\"\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("WorkingDirectory=/home/u/my \"q\" 'x' 50%% $HOME ${X} ö ş/\n"),
            "{text}"
        );
        assert!(text.contains("Description=100%% sure\n"), "{text}");
        // One line per setting: nothing a value holds starts a new one.
        assert_eq!(text.lines().count(), 15, "{text}");
    }

    #[test]
    fn a_line_break_a_nul_a_relative_path_or_bytes_that_are_not_utf8_are_refused() {
        let base = spec(Path::new("/home/u/.agora"));
        let refused = |s: &ServiceSpec| {
            assert_eq!(
                unit_text(s).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            service_refusal(s).unwrap().1
        };
        let mut s = base.clone();
        s.args[3] = "/home/u/a\nExecStartPre=/bin/evil".into();
        assert_eq!(refused(&s), ServiceRefusal::ControlCharacter);
        let mut s = base.clone();
        s.args[3] = OsString::from_vec(b"/home/u/a\0b".to_vec());
        assert_eq!(refused(&s), ServiceRefusal::ControlCharacter);
        let mut s = base.clone();
        s.program = PathBuf::from("agora");
        assert_eq!(refused(&s), ServiceRefusal::NotAbsolute);
        let mut s = base.clone();
        s.work_dir = PathBuf::from("rel/home");
        assert_eq!(refused(&s), ServiceRefusal::NotAbsolute);
        let mut s = base.clone();
        s.work_dir = PathBuf::from("/home/u/trailing ");
        assert_eq!(refused(&s), ServiceRefusal::CannotCarry);
        let mut s = base.clone();
        s.args[3] = OsString::from_vec(b"/home/u/\xff".to_vec());
        assert_eq!(refused(&s), ServiceRefusal::CannotCarry);
        assert_eq!(service_refusal(&base), None);
        // `%` is written `%%` here, not refused (Windows refuses it).
        let mut s = base;
        s.args[3] = "/home/u/50%".into();
        assert_eq!(service_refusal(&s), None);
    }

    #[test]
    fn only_full_path_entries_of_path_go_into_the_unit_in_their_order() {
        let mut s = spec(Path::new("/h"));
        s.env = vec![
            ("HOME".into(), "/elsewhere".into()),
            (
                "PATH".into(),
                "/usr/bin::relative:./x:/a b/%x:/c\"d:/e$F:".into(),
            ),
            ("SECRET".into(), "token".into()),
        ];
        let text = unit_text(&s).unwrap();
        assert!(
            text.contains("Environment=\"PATH=/usr/bin:/a b/%%x:/c\\\"d:/e$F\"\n"),
            "{text}"
        );
        assert_eq!(text.matches("Environment=").count(), 1, "{text}");
        assert!(!text.contains("SECRET") && !text.contains("/elsewhere"));
        s.env = vec![("PATH".into(), ":relative:".into())];
        assert!(!unit_text(&s).unwrap().contains("Environment="));
    }

    #[test]
    fn a_unit_agora_wrote_is_rewritten_when_its_text_changed_and_one_it_did_not_write_never() {
        let f = Fake::new("rewrite");
        let mut s = spec(&f.dir);
        f.systemd.install(&s).unwrap();
        let unit = f.unit();
        // Same text: nothing differs and the file is not rewritten.
        assert!(!f.systemd.differs(&s).unwrap());
        let inode = fs::metadata(&unit).unwrap().ino();
        f.systemd.install(&s).unwrap();
        assert_eq!(fs::metadata(&unit).unwrap().ino(), inode);
        // A newer setting: the file Agora wrote is replaced (owner-only) and systemd reloaded, never restarted.
        s.keep_children_on_stop = true;
        assert!(f.systemd.differs(&s).unwrap());
        f.systemd.install(&s).unwrap();
        let text = fs::read_to_string(&unit).unwrap();
        assert!(text.contains("KillMode=process\n"), "{text}");
        assert_eq!(fs::metadata(&unit).unwrap().mode() & 0o777, 0o600);
        assert!(!f.systemd.differs(&s).unwrap());
        assert!(f.log().ends_with("--user enable agora-node.service\n"));
        assert!(!f.log().contains("restart"));
        // A file without Agora's first line is refused and left as it is.
        fs::write(&unit, "[Service]\nExecStart=/bin/true\n").unwrap();
        s.keep_children_on_stop = false;
        let e = f.systemd.install(&s).unwrap_err();
        assert_eq!(
            crate::service_problem(&e),
            Some(&ServiceProblem::NotOurs(unit.clone()))
        );
        assert_eq!(
            fs::read_to_string(&unit).unwrap(),
            "[Service]\nExecStart=/bin/true\n"
        );
    }

    #[test]
    fn install_writes_a_private_unit_once_and_registers_it_without_starting_it() {
        let f = Fake::new("install");
        let s = spec(&f.dir);
        f.systemd.install(&s).unwrap();
        let unit = f.unit();
        assert_eq!(fs::metadata(&unit).unwrap().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(f.dir.join("units")).unwrap().mode() & 0o777,
            0o700
        );
        assert!(fs::read_to_string(&unit).unwrap().contains("ExecStart="));
        assert_eq!(
            f.log(),
            "--user daemon-reload\n--user show -p FragmentPath -p DropInPaths agora-node.service\n\
             --user enable agora-node.service\n"
        );
        assert_eq!(
            f.systemd.status("agora-node").unwrap(),
            ServiceState::Installed { enabled: true }
        );
        // The same unit again: the file is not rewritten.
        let inode = fs::metadata(&unit).unwrap().ino();
        f.systemd.install(&s).unwrap();
        assert_eq!(fs::metadata(&unit).unwrap().ino(), inode);
        f.systemd.start("agora-node").unwrap();
        assert!(f.log().ends_with("--user start agora-node.service\n"));
        assert!(!f.log().contains("--now"));
    }

    #[test]
    fn a_link_at_the_units_place_is_refused_and_its_target_left_alone() {
        let f = Fake::new("link");
        fs::create_dir_all(f.dir.join("units")).unwrap();
        let target = f.dir.join("target");
        fs::write(&target, "mine").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        std::os::unix::fs::symlink(&target, f.unit()).unwrap();
        let e = f.systemd.install(&spec(&f.dir)).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            crate::service_problem(&e),
            Some(&ServiceProblem::InTheWay(f.unit()))
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "mine");
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o644);
        assert!(fs::symlink_metadata(f.unit())
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(!f.log().contains("enable"));
    }

    #[test]
    fn a_failed_registration_takes_its_new_unit_back() {
        let f = Fake::new("rollback");
        f.mode("failenable");
        let e = f.systemd.install(&spec(&f.dir)).unwrap_err();
        assert!(e.to_string().contains("boom"), "{e}");
        assert!(!f.unit().exists());
    }

    #[test]
    fn remove_twice_or_after_the_unit_was_deleted_by_hand_is_safe() {
        let f = Fake::new("remove");
        assert!(!f.systemd.remove("agora-node").unwrap());
        f.systemd.install(&spec(&f.dir)).unwrap();
        fs::write(f.dir.join("log"), "").unwrap();
        assert!(f.systemd.remove("agora-node").unwrap());
        assert!(!f.unit().exists());
        assert_eq!(
            f.log(),
            "--user is-enabled agora-node.service\n--user disable --now agora-node.service\n--user daemon-reload\n"
        );
        assert!(!f.systemd.remove("agora-node").unwrap());
        assert_eq!(
            f.systemd.status("agora-node").unwrap(),
            ServiceState::NotInstalled
        );
        // Deleted by hand: nothing registered is found, and nothing is touched.
        f.systemd.install(&spec(&f.dir)).unwrap();
        fs::remove_file(f.unit()).unwrap();
        assert!(!f.systemd.remove("agora-node").unwrap());
    }

    #[test]
    fn status_tells_not_found_from_a_missing_user_bus_and_other_failures() {
        let f = Fake::new("status");
        assert_eq!(
            f.systemd.status("agora-node").unwrap(),
            ServiceState::NotInstalled
        );
        f.mode("new");
        assert_eq!(
            f.systemd.status("agora-node").unwrap(),
            ServiceState::NotInstalled
        );
        f.mode("");
        f.systemd.install(&spec(&f.dir)).unwrap();
        fs::remove_file(f.dir.join("enabled-agora-node.service")).unwrap();
        assert_eq!(
            f.systemd.status("agora-node").unwrap(),
            ServiceState::Installed { enabled: false }
        );
        f.mode("bus");
        let e = f.systemd.status("agora-node").unwrap_err();
        assert_eq!(
            crate::service_problem(&e),
            Some(&ServiceProblem::NoUserManager)
        );
        // A failing manager is never taken for "not installed", and remove/install stop there too.
        assert!(f.systemd.remove("agora-node").is_err());
        assert!(f.unit().exists());
        f.mode("broken");
        let e = f.systemd.status("agora-node").unwrap_err();
        assert_eq!(crate::service_problem(&e), None);
        assert!(e.to_string().contains("Access denied"), "{e}");
    }

    /// A unit of the same name written by hand (as on the maintainer's own machine) is never replaced, turned off or deleted.
    #[test]
    fn a_unit_agora_did_not_write_is_left_as_it_is() {
        let f = Fake::new("theirs");
        fs::create_dir_all(f.dir.join("units")).unwrap();
        let theirs = "[Unit]\nDescription=Local agent node\n\n[Service]\nExecStart=/usr/bin/agora node run\n";
        fs::write(f.unit(), theirs).unwrap();
        fs::write(f.dir.join("enabled-agora-node.service"), "").unwrap();
        assert_eq!(f.systemd.foreign("agora-node").unwrap(), Some(f.unit()));
        let e = f.systemd.remove("agora-node").unwrap_err();
        assert_eq!(
            crate::service_problem(&e),
            Some(&ServiceProblem::NotOurs(f.unit()))
        );
        let e = f.systemd.install(&spec(&f.dir)).unwrap_err();
        assert_eq!(
            crate::service_problem(&e),
            Some(&ServiceProblem::NotOurs(f.unit()))
        );
        assert_eq!(fs::read_to_string(f.unit()).unwrap(), theirs);
        assert!(f.dir.join("enabled-agora-node.service").exists());
        assert!(!f.log().contains("disable") && !f.log().contains("daemon-reload"));
        // Registered from another folder: foreign too, named by the unit.
        fs::remove_file(f.unit()).unwrap();
        assert_eq!(f.systemd.foreign("agora-node").unwrap(), None);
        f.systemd.install(&spec(&f.dir)).unwrap();
        assert_eq!(f.systemd.foreign("agora-node").unwrap(), None);
    }

    /// A drop-in left from a hand-made unit (`agora-node.service.d/`, `agora-.service.d/`) would change what Agora's unit runs: the
    /// install is refused and taken back, nothing is enabled; `drop_ins` names them for `remove`.
    #[test]
    fn a_drop_in_or_another_unit_file_refuses_the_install_and_takes_it_back() {
        for dir in ["agora-node.service.d", "agora-.service.d"] {
            let f = Fake::new(&format!("dropin-{dir}"));
            let d = f.dir.join("units").join(dir);
            fs::create_dir_all(&d).unwrap();
            fs::write(
                d.join("10-local-build.conf"),
                "[Service]\nExecStart=\nExecStart=/x\n",
            )
            .unwrap();
            let e = f.systemd.install(&spec(&f.dir)).unwrap_err();
            assert_eq!(
                crate::service_problem(&e),
                Some(&ServiceProblem::Overridden(vec![
                    d.join("10-local-build.conf")
                ]))
            );
            assert!(!f.unit().exists(), "the unit was not taken back");
            assert!(!f.log().contains("enable"), "{}", f.log());
            assert!(f.log().ends_with("--user daemon-reload\n"), "{}", f.log());
            assert_eq!(
                f.systemd.drop_ins("agora-node").unwrap(),
                vec![d.join("10-local-build.conf")]
            );
        }
        // systemd reads its units from another folder: refused, named, taken back.
        let f = Fake::new("elsewhere");
        f.mode("elsewhere");
        let e = f.systemd.install(&spec(&f.dir)).unwrap_err();
        assert_eq!(
            crate::service_problem(&e),
            Some(&ServiceProblem::NotLoaded(f.unit()))
        );
        assert!(!f.unit().exists());
    }

    /// The executable is the first word: systemd expands no variable there and does not undo `$$`, so `$` stays as it is; `%` is still
    /// a specifier; a quote or a backslash makes systemd refuse it, so Agora refuses it first.
    #[test]
    fn the_program_path_is_written_as_systemd_runs_it() {
        let mut s = spec(Path::new("/home/u/h"));
        s.program = PathBuf::from("/opt/d$X/p%q/my apps/agora");
        s.args[1] = "n$o%de".into();
        let text = unit_text(&s).unwrap();
        assert!(
            text.contains("ExecStart=\"/opt/d$X/p%%q/my apps/agora\" \"supervise\" \"n$$o%%de\" "),
            "{text}"
        );
        for bad in ["/opt/q\"x/agora", "/opt/a'b/agora", "/opt/b\\c/agora"] {
            let mut s = spec(Path::new("/home/u/h"));
            s.program = PathBuf::from(bad);
            assert_eq!(
                service_refusal(&s).unwrap().1,
                ServiceRefusal::ProgramCharacter,
                "{bad}"
            );
            assert!(unit_text(&s).is_err());
        }
    }

    /// The same unit checked by systemd itself, offline (`systemd-analyze verify` in a scratch folder: it loads the file and starts
    /// nothing). Skipped, with a note, where systemd-analyze is not installed.
    #[test]
    fn systemd_itself_accepts_the_unit_with_odd_paths() {
        let Some(analyze) = std::env::var_os("PATH").and_then(|p| {
            std::env::split_paths(&p)
                .filter(|d| d.is_absolute())
                .map(|d| d.join("systemd-analyze"))
                .find(|p| p.is_file())
        }) else {
            eprintln!("note: systemd-analyze not found; the unit was not checked by systemd");
            return;
        };
        let f = Fake::new("analyze");
        let bin = f.dir.join("d$X p%q");
        fs::create_dir_all(&bin).unwrap();
        let program = bin.join("agora");
        fs::write(&program, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        let home = f.dir.join("home 50% $HOME 'q' \"d\"");
        fs::create_dir_all(&home).unwrap();
        let mut s = spec(&home);
        s.program = program;
        s.env = vec![("PATH".into(), "/usr/bin:/a b/%x:/c\"d".into())];
        let unit = f.dir.join("agora-analyze.service");
        fs::write(&unit, unit_text(&s).unwrap()).unwrap();
        let scratch = f.dir.join("scratch");
        fs::create_dir_all(&scratch).unwrap();
        let out = std::process::Command::new(analyze)
            .args(["verify", "--man=no", "--generators=no"])
            .arg(&unit)
            .env_clear()
            .env("HOME", &scratch)
            .env("XDG_CONFIG_HOME", scratch.join("cfg"))
            .env("XDG_RUNTIME_DIR", scratch.join("run"))
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        let said = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "{said}");
        assert!(
            !said.contains("not executable") && !said.contains("special characters"),
            "{said}"
        );
    }

    #[test]
    fn a_drop_in_path_with_a_space_stays_whole() {
        assert_eq!(
            split_drop_ins("/x/my dir/a.conf /y/b.conf /z/two  spaces.conf"),
            vec![
                PathBuf::from("/x/my dir/a.conf"),
                PathBuf::from("/y/b.conf"),
                PathBuf::from("/z/two  spaces.conf")
            ]
        );
        assert!(split_drop_ins("").is_empty());
    }

    #[test]
    fn show_is_read_by_its_keys() {
        assert_eq!(
            read_show("DropInPaths=/a/x.conf /b/y.conf\nFragmentPath=/u/agora-node.service\n"),
            (
                Some(PathBuf::from("/u/agora-node.service")),
                vec![PathBuf::from("/a/x.conf"), PathBuf::from("/b/y.conf")]
            )
        );
        assert_eq!(
            read_show("FragmentPath=\nDropInPaths=\n"),
            (None, Vec::new())
        );
    }

    /// Only `enabled` starts at login; every other state systemctl can answer is present but not starting by itself.
    #[test]
    fn each_is_enabled_answer_maps_to_one_state() {
        use std::os::unix::process::ExitStatusExt;
        let out = |word: &str, code: i32, err: &str| std::process::Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: if word.is_empty() {
                Vec::new()
            } else {
                format!("{word}\n").into_bytes()
            },
            stderr: err.as_bytes().to_vec(),
        };
        for w in ["enabled", "enabled-runtime"] {
            assert_eq!(
                read_is_enabled(&out(w, 0, "")).unwrap(),
                ServiceState::Installed { enabled: true },
                "{w}"
            );
        }
        for w in [
            "disabled",
            "static",
            "indirect",
            "generated",
            "alias",
            "transient",
            "masked",
            "masked-runtime",
            "linked",
            "linked-runtime",
            "bad",
        ] {
            let code = if w == "disabled" || w.starts_with("masked") || w == "bad" {
                1
            } else {
                0
            };
            assert_eq!(
                read_is_enabled(&out(w, code, "")).unwrap(),
                ServiceState::Installed { enabled: false },
                "{w}"
            );
        }
        assert_eq!(
            read_is_enabled(&out("not-found", 4, "")).unwrap(),
            ServiceState::NotInstalled
        );
        assert_eq!(
            read_is_enabled(&out(
                "",
                1,
                "Failed to get unit file state for x.service: No such file or directory"
            ))
            .unwrap(),
            ServiceState::NotInstalled
        );
        let e =
            read_is_enabled(&out("", 1, "Failed to connect to bus: No medium found")).unwrap_err();
        assert_eq!(
            crate::service_problem(&e),
            Some(&ServiceProblem::NoUserManager)
        );
        assert!(read_is_enabled(&out("", 1, "Access denied")).is_err());
        assert!(read_is_enabled(&out("something-new", 0, "")).is_err());
    }

    /// A folder with a backslash is refused (`WorkingDirectory=` is unquoted; a trailing one would join the next line).
    #[test]
    fn a_backslash_in_the_folder_is_refused() {
        for home in ["/home/u/home\\", "/home/u/a\\b"] {
            let s = spec(Path::new(home));
            assert_eq!(
                service_refusal(&s).unwrap().1,
                ServiceRefusal::Backslash,
                "{home}"
            );
            assert_eq!(
                unit_text(&s).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        // In an argument, inside its quotes, it is escaped.
        let mut s = spec(Path::new("/home/u/h"));
        s.args[1] = "a\\b".into();
        assert!(unit_text(&s).unwrap().contains(" \"a\\\\b\" "));
    }

    /// Root (`sudo`) is refused before anything is written, and so is a missing HOME, each with its own problem.
    #[test]
    fn root_and_a_missing_home_are_refused() {
        let e = unit_dir_for(0, Some("/h/.config".into()), Some("/h".into())).unwrap_err();
        assert_eq!(crate::service_problem(&e), Some(&ServiceProblem::Root));
        let e = unit_dir_for(1000, None, None).unwrap_err();
        assert_eq!(
            crate::service_problem(&e),
            Some(&ServiceProblem::NoConfigFolder)
        );
        let e = unit_dir_for(1000, Some("rel".into()), Some("also-rel".into())).unwrap_err();
        assert_eq!(
            crate::service_problem(&e),
            Some(&ServiceProblem::NoConfigFolder)
        );
        assert_eq!(
            unit_dir_for(1000, Some("rel".into()), Some("/h".into())).unwrap(),
            PathBuf::from("/h/.config/systemd/user")
        );
        assert_eq!(
            unit_dir_for(1000, Some("/x".into()), Some("/h".into())).unwrap(),
            PathBuf::from("/x/systemd/user")
        );
    }

    /// `systemctl` is run by the full path found in a full-path folder of `PATH`: empty, `.` and relative entries are skipped.
    #[test]
    fn systemctl_is_found_only_in_full_path_folders() {
        let f = Fake::new("lookup");
        let planted = f.dir.join("rel");
        fs::create_dir_all(&planted).unwrap();
        fs::copy(f.dir.join("systemctl"), planted.join("systemctl")).unwrap();
        // Relative entries first: only the full-path folder counts.
        let path = std::env::join_paths([
            PathBuf::from(""),
            PathBuf::from("."),
            PathBuf::from("rel"),
            f.dir.clone(),
        ])
        .unwrap();
        assert_eq!(systemctl_in(&path), Some(f.dir.join("systemctl")));
        let only_relative =
            std::env::join_paths([PathBuf::from("rel"), PathBuf::from("")]).unwrap();
        assert_eq!(systemctl_in(&only_relative), None);
        // A file that is not executable is not taken.
        let plain = f.dir.join("plain");
        fs::create_dir_all(&plain).unwrap();
        fs::write(plain.join("systemctl"), "").unwrap();
        assert_eq!(systemctl_in(plain.as_os_str()), None);
    }

    #[test]
    fn a_unit_name_is_one_plain_word() {
        let f = Fake::new("name");
        for bad in ["", "../x", "a b", "a/b", ".hidden", "a\nb"] {
            assert_eq!(
                f.systemd.status(bad).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{bad:?}"
            );
        }
        assert_eq!(f.log(), "");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod held_folder_rule_tests {
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};

    /// F1022: a rule for the held folder, or for a path below it, names the held folder even while its path leads to another
    /// folder (swapped for a link while the rules are made, swapped back before the start). The process is confined with those
    /// rules and tries to write in both folders.
    #[test]
    fn rules_at_or_below_the_held_folder_name_it_whatever_its_path_leads_to() {
        if !super::sandbox_supported() {
            eprintln!("skipped: no Landlock on this machine");
            return;
        }
        let base = std::env::temp_dir().join(format!("agora-held-rules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let base = std::fs::canonicalize(&base).unwrap();
        let (own, other) = (base.join("own/x"), base.join("other/x"));
        for d in [&own, &other] {
            std::fs::create_dir_all(d.join("sub")).unwrap();
        }
        let system: Vec<PathBuf> = ["/usr", "/bin", "/lib", "/lib64", "/etc"]
            .iter()
            .map(PathBuf::from)
            .collect();
        // Codex review: the folder named through a link to `base` (the spelling a start was given; the hold resolved it).
        let alias = base.join("alias");
        std::os::unix::fs::symlink(&base, &alias).unwrap();
        let spelled = alias.join("own/x");
        // Whether a confined `sh` may write `file`: one process per file, so nothing it did before decides.
        let writes = |write: &Path, file: &Path| {
            let held = crate::HeldFolder::open(&own).unwrap();
            std::fs::rename(&own, own.with_file_name("x.old")).unwrap();
            std::os::unix::fs::symlink(&other, &own).unwrap();
            let sb = crate::Sandbox::new(system.clone(), vec![write.to_path_buf()]);
            let rules = super::ruleset(&sb, Some(&held), Some(&spelled)).unwrap();
            std::fs::remove_file(&own).unwrap();
            std::fs::rename(own.with_file_name("x.old"), &own).unwrap();
            let mut cmd = std::process::Command::new("/bin/sh");
            cmd.args(["-c", "echo x > \"$1\"", "sh"]).arg(file);
            // SAFETY: the closure only applies a ruleset made before the fork (async-signal-safe calls).
            unsafe { cmd.pre_exec(move || rules.enforce()) };
            let ok = cmd.status().unwrap().success();
            let _ = std::fs::remove_file(file);
            ok
        };
        for write in [
            own.clone(),
            own.join("sub"),
            spelled.clone(),
            spelled.join("sub"),
        ] {
            assert!(writes(&write, &own.join("sub/f")), "{write:?}");
            assert!(!writes(&write, &other.join("sub/f")), "{write:?}");
        }
        // A spelling inside the held folder (a link there to the folder itself), retargeted while the rules are made: entries below
        // it are the held folder's, not what the link leads to by then.
        let inner = own.join("self");
        std::os::unix::fs::symlink(&own, &inner).unwrap();
        let held = crate::HeldFolder::open(&own).unwrap();
        std::fs::remove_file(&inner).unwrap();
        std::os::unix::fs::symlink(&other, &inner).unwrap();
        let sb = crate::Sandbox::new(system.clone(), vec![inner.join("sub")]);
        let rules = super::ruleset(&sb, Some(&held), Some(&inner)).unwrap();
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.args(["-c", "echo x > \"$1\" && echo x > \"$2\"", "sh"])
            .arg(own.join("sub/f"))
            .arg(other.join("sub/f"));
        // SAFETY: the closure only applies a ruleset made before the fork (async-signal-safe calls).
        unsafe { cmd.pre_exec(move || rules.enforce()) };
        assert!(!cmd.status().unwrap().success());
        assert!(own.join("sub/f").exists(), "the held folder's own rule");
        assert!(!other.join("sub/f").exists(), "the retargeted link's rule");
        let _ = std::fs::remove_dir_all(base);
    }

    /// Codex review of F1021: a folder the user may enter but not list (search permission only) can be held, is in place, and a
    /// process started in it runs there.
    #[test]
    fn a_folder_that_can_be_entered_but_not_listed_can_be_held_and_started_in() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("agora-held-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("x")).unwrap();
        let base = std::fs::canonicalize(&base).unwrap();
        let (dir, out) = (base.join("x"), base.join("out"));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o300)).unwrap();
        let held = crate::HeldFolder::open(&dir);
        let mut ran = String::new();
        if let Ok(held) = &held {
            assert!(held.in_place());
            let mut cfg = crate::PtyConfig::new(vec![
                "/bin/sh".into(),
                "-c".into(),
                format!("pwd -P > '{}'", out.display()),
            ]);
            cfg.cwd = Some(dir.clone());
            let mut pty = crate::spawn_in(&cfg, held).unwrap();
            let _ = pty.wait();
            ran = std::fs::read_to_string(&out).unwrap_or_default();
        }
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let _ = std::fs::remove_dir_all(&base);
        assert!(held.is_ok(), "{held:?}");
        assert_eq!(ran.trim(), dir.to_str().unwrap());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod closed_grant_tests {
    use std::path::{Path, PathBuf};

    /// The refusal a ruleset was made with, if any (`None`: it was made).
    fn refusal(sb: &crate::Sandbox, held: Option<&crate::HeldFolder>) -> Option<String> {
        match super::ruleset(sb, held, None) {
            Ok(_) => None,
            Err(e) => Some(
                crate::sandbox_refusal(&e)
                    .map_or_else(|| format!("not a refusal: {e}"), |r| r.token().to_string()),
            ),
        }
    }

    fn grant(write: &Path, closed: &[PathBuf]) -> crate::Sandbox {
        let mut sb = crate::Sandbox::new(vec![], vec![write.to_path_buf()]);
        sb.closed = closed.to_vec();
        sb
    }

    /// #1150: the node checks the grants by name before the start, then Landlock opens them again. A grant that is a link by then
    /// (swapped in a place another agent may write) into the node's folder, to its data folder or to its control token is refused
    /// on what was opened; a link elsewhere, a real folder and a folder below a closed one (the agent's worktree) are granted.
    #[test]
    fn a_grant_that_leads_to_or_above_a_closed_path_when_opened_is_refused() {
        if !super::sandbox_supported() {
            eprintln!("skipped: no Landlock on this machine");
            return;
        }
        let base = std::env::temp_dir().join(format!("agora-closed-grant-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let base = std::fs::canonicalize(&base).unwrap();
        let (home, data, worktree, shared, elsewhere) = (
            base.join("node"),
            base.join("node/data"),
            base.join("node/worktrees/w"),
            base.join("shared"),
            base.join("elsewhere"),
        );
        for d in [&data, &worktree, &shared, &elsewhere] {
            std::fs::create_dir_all(d).unwrap();
        }
        let token = home.join("control.token");
        std::fs::write(&token, "t").unwrap();
        let closed = vec![
            token.clone(),
            home.clone(),
            data.clone(),
            base.join("node/missing/deeper"),
        ];
        let link = shared.join("grant");
        let point = |to: &Path| {
            let _ = std::fs::remove_file(&link);
            let _ = std::fs::remove_dir(&link);
            std::os::unix::fs::symlink(to, &link).unwrap();
        };
        let refused = Some("protected_path_reachable".to_string());
        for to in [&home, &data, &token, &base] {
            point(to);
            assert_eq!(refusal(&grant(&link, &closed), None), refused, "{to:?}");
        }
        // Only the missing path closed: it counts from its nearest existing folder.
        point(&home);
        let missing = vec![base.join("node/missing/deeper")];
        assert_eq!(refusal(&grant(&link, &missing), None), refused);
        // A link elsewhere, a real folder and a folder below a closed one are granted, as before.
        point(&elsewhere);
        assert_eq!(refusal(&grant(&link, &closed), None), None);
        assert_eq!(refusal(&grant(&shared, &closed), None), None);
        assert_eq!(refusal(&grant(&worktree, &closed), None), None);
        // A relative closed path (here a missing one) counts from the current folder.
        let cwd = std::env::current_dir().unwrap();
        let local = cwd.join(format!("agora-closed-local-{}", std::process::id()));
        point(&cwd);
        assert_eq!(
            refusal(&grant(&link, &[local.file_name().unwrap().into()]), None),
            refused,
            "a missing relative path counts from the current folder"
        );
        // Nothing closed: nothing is compared.
        point(&home);
        assert_eq!(refusal(&grant(&link, &[]), None), None);
        // An entry below the held folder is opened through its handle (F1022) and compared the same way.
        let held = crate::HeldFolder::open(&shared).unwrap();
        assert_eq!(refusal(&grant(&link, &closed), Some(&held)), refused);
        // A closed path spelled through a link on the way (`spelled/via/data`, `via` a link to `elsewhere`): the folder above it by
        // name (`spelled`) is refused too, as the node's own check compares names, and so is where it leads.
        let spelled = base.join("spelled");
        std::fs::create_dir_all(&spelled).unwrap();
        std::os::unix::fs::symlink(&elsewhere, spelled.join("via")).unwrap();
        std::fs::create_dir_all(elsewhere.join("data")).unwrap();
        let through = vec![spelled.join("via/data")];
        for to in [&spelled, &elsewhere] {
            point(to);
            assert_eq!(refusal(&grant(&link, &through), None), refused, "{to:?}");
        }
        // A closed path that cannot be looked at (a loop of links on the way, not a missing name) refuses the start: it never
        // counts from a folder further up.
        let ring = base.join("ring");
        std::os::unix::fs::symlink(&ring, &ring).unwrap();
        point(&elsewhere);
        assert_eq!(
            refusal(&grant(&link, &[ring.join("data")]), None),
            Some("protected_path_unresolved".to_string())
        );
        // The node's generated Git file (an exact file below its data folder) is still read.
        let git = data.join("gitconfig");
        std::fs::write(&git, "").unwrap();
        let mut sb = grant(&worktree, &closed);
        sb.git_config = Some(git.clone());
        assert_eq!(
            super::ruleset(&sb, None, None).unwrap().git_config,
            Some(git)
        );
        let _ = std::fs::remove_dir_all(base);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod held_output_tests {
    use std::time::{Duration, Instant};

    use crate::{HeldBuffer, HeldTerminal, ProcessLimits, PtyConfig};

    /// A program that writes `bye` and ends at once, collected by the holder before anything read its terminal.
    fn ended_unread() -> HeldTerminal {
        let mut t = HeldTerminal::start(&PtyConfig {
            argv: vec!["printf".into(), "bye".into()],
            cwd: None,
            env: vec![("PATH".into(), std::env::var("PATH").unwrap_or_default())],
            cols: 80,
            rows: 24,
            limits: ProcessLimits {
                cpu_seconds: None,
                memory_bytes: None,
            },
            sandbox: None,
            identity: None,
            env_remove: Vec::new(),
        })
        .unwrap();
        let end = Instant::now() + Duration::from_secs(10);
        while t.try_wait().unwrap().is_none() {
            assert!(Instant::now() < end, "it did not end");
            std::thread::sleep(Duration::from_millis(1));
        }
        t
    }

    /// The holder may collect an agent's end before it reads the agent's last output: Linux keeps that output readable on the master
    /// after the last close of the terminal's program side, so it is kept whichever way it is read (the holder's reader to the end;
    /// a drain alone; a reader ended at once, then a drain, as `attach` does). macOS keeps it only 0.6 s (holder-link.md decision 5),
    /// so this holds on Linux only.
    #[test]
    fn output_left_by_a_collected_agent_is_read_in_every_order() {
        for _ in 0..30 {
            let t = ended_unread();
            let r = t.start_reader().unwrap();
            let end = Instant::now() + Duration::from_secs(10);
            while !r.ended() {
                assert!(Instant::now() < end, "the reader never saw the end");
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(r.end().buffer.into_vec(), b"bye", "reader");

            let t = ended_unread();
            let mut b = HeldBuffer::default();
            assert!(t.drain(&mut b).unwrap(), "the drain saw the end");
            assert_eq!(b.into_vec(), b"bye", "drain");

            let t = ended_unread();
            let c = t.start_reader().unwrap().end();
            let mut b = c.buffer;
            if !c.ended {
                assert!(t.drain(&mut b).unwrap(), "the drain saw the end");
            }
            assert_eq!(b.into_vec(), b"bye", "reader ended at once, then a drain");
        }
    }
}
