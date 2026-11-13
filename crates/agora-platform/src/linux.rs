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
const READ_FS: u64 = 1 | (1 << 2) | (1 << 3);
/// What applies to a file rather than a directory: execute, write and read.
const FILE_FS: u64 = 1 | (1 << 1) | (1 << 2);

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
}

#[repr(C, packed)]
struct PathBeneath {
    allowed_access: u64,
    parent_fd: i32,
}

/// True when the running kernel has Landlock switched on.
pub(crate) fn sandbox_supported() -> bool {
    // SAFETY: the version query takes a null attribute and size 0.
    let v = unsafe {
        libc::syscall(
            SYS_LANDLOCK_CREATE_RULESET,
            std::ptr::null::<RulesetAttr>(),
            0usize,
            CREATE_RULESET_VERSION,
        )
    };
    v >= 1
}

/// A ruleset made before the fork, so the child only has to apply it (two async-signal-safe calls).
pub(crate) struct Ruleset(std::os::fd::OwnedFd);

pub(crate) fn ruleset(sb: &crate::Sandbox) -> io::Result<Ruleset> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    if !sandbox_supported() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this kernel has no Landlock",
        ));
    }
    let attr = RulesetAttr {
        handled_access_fs: ALL_FS,
    };
    // SAFETY: a valid attribute pointer and its size; the result is a new descriptor (close-on-exec) or -1.
    let fd = unsafe {
        libc::syscall(
            SYS_LANDLOCK_CREATE_RULESET,
            &attr as *const RulesetAttr,
            std::mem::size_of::<RulesetAttr>(),
            0 as libc::c_ulong,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the kernel returned a fresh descriptor that nothing else owns.
    let set = unsafe { OwnedFd::from_raw_fd(fd as i32) };
    for (paths, rights) in [(&sb.read, READ_FS), (&sb.write, ALL_FS)] {
        for p in paths {
            // A path that does not exist on this machine confines nothing; skip it.
            let Ok(f) = std::fs::File::open(p) else {
                continue;
            };
            let rights = if f.metadata()?.is_dir() {
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
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(Ruleset(set))
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
                self.0.as_raw_fd(),
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
