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
// - The unit runs the supervisor in the foreground (`Type=simple`); systemd's default `KillMode=control-group` ends everything the
//   role started when the unit stops, after `TimeoutStopSec=25`. `Restart=on-failure` (when the spec asks for a restart) starts again
//   a supervisor that crashed, was killed by a signal (an OOM kill) or timed out. Under a service (`--log`) its planned ends (give-up,
//   refusal, lost lock, clean stop) exit 0 and are never restarted (decision `2026-11-18-1100-linux-563`). Nothing else restarts.
// - The unit file is written owner-only (0600) and atomically into `$XDG_CONFIG_HOME/systemd/user` (`~/.config/systemd/user`),
//   never through a link at its own place. `ExecStart` is quoted for systemd itself (no shell): every word in double quotes, `\` and
//   `"` escaped, `%` written `%%` (a specifier) and `$` written `$$` (a variable). Only `PATH` goes into `Environment=`.
// - `systemctl` is run with `LC_ALL=C`, so its answers can be read; the reason of a failure is passed on, never parsed further.

/// The first line of every unit Agora writes. A unit at the same place without it (written by hand, by another tool) is never
/// replaced, disabled or deleted: [`crate::ServiceProblem::NotOurs`].
const UNIT_MARKER: &str = "# Login service (ADR-0062 item 6): written and removed by its own service command; edits are not kept.\n";

/// See [`crate::services_available`]: an executable `systemctl` in a full-path folder of `PATH`.
pub(crate) fn services_available() -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p)
            .filter(|d| d.is_absolute())
            .any(|d| {
                fs::metadata(d.join("systemctl"))
                    .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            })
    })
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
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .map(|h| h.join(".config"))
            })
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "neither XDG_CONFIG_HOME nor HOME is set",
                )
            })?;
        Ok(Systemd {
            systemctl: PathBuf::from("systemctl"),
            unit_dir: config.join("systemd").join("user"),
        })
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
        let new = fs::symlink_metadata(&path).is_err();
        write_unit(&self.unit_dir, &path, &text)?;
        let registered = self
            .checked(&["daemon-reload"])
            .and_then(|()| self.checked(&["enable", &unit_name(&spec.name)]));
        if registered.is_err() && new {
            // Leave nothing half-registered behind: a unit that was not there before is taken back.
            let _ = fs::remove_file(&path);
            let _ = self.checked(&["daemon-reload"]);
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
        "enabled" | "enabled-runtime" | "static" | "indirect" | "generated" | "alias"
        | "transient" => Ok(Installed { enabled: true }),
        // Present, but it does not start by itself: turned off (`disable`, `mask`) or linked in by hand.
        "disabled" | "linked" | "linked-runtime" | "masked" | "masked-runtime" | "bad" => {
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
        if folder && t != t.trim() {
            return Err(CannotCarry);
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

/// A word of `ExecStart=`: in double quotes, with `\` and `"` escaped (systemd unescapes C style), `%` as `%%` (a specifier) and `$`
/// as `$$` (a variable). No shell reads it.
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

/// The `PATH` the unit gets: the spec's, keeping the order but only full paths that the unit can carry (an empty or relative entry
/// would mean "the folder systemd runs it from"). No other variable of the installing shell goes into the unit.
fn unit_path_variable(env: &[(String, std::ffi::OsString)]) -> Option<String> {
    let (_, value) = env.iter().find(|(k, _)| k == "PATH")?;
    let kept: Vec<String> = std::env::split_paths(value)
        .filter(|p| p.is_absolute())
        .filter_map(|p| p.to_str().map(str::to_owned))
        .filter(|p| !p.chars().any(char::is_control))
        .collect();
    (!kept.is_empty()).then(|| kept.join(":"))
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
    let mut exec = exec_word(spec.program.to_str().ok_or_else(invalid)?);
    for a in &spec.args {
        exec.push(' ');
        exec.push_str(&exec_word(a.to_str().ok_or_else(invalid)?));
    }
    let description: String = spec
        .description
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .replace('%', "%%");
    let mut unit = format!(
        "{UNIT_MARKER}[Unit]\nDescription={description}\n\n[Service]\nType=simple\nExecStart={exec}\nWorkingDirectory={}\n",
        work_dir.replace('%', "%%")
    );
    if let Some(path) = unit_path_variable(&spec.env) {
        unit.push_str(&format!(
            "Environment={}\n",
            env_word(&format!("PATH={path}"))
        ));
    }
    unit.push_str("KillMode=control-group\nTimeoutStopSec=25\n");
    if spec.restart_supervisor_on_failure {
        unit.push_str("Restart=on-failure\n");
    }
    unit.push_str("\n[Install]\nWantedBy=default.target\n");
    Ok(unit)
}

/// Writes the unit at `path` in `dir` (made 0700 when missing), owner-only and atomically; `Ok(false)` when the same text is there
/// already. A link or anything but a plain file at `path` is refused, never followed or replaced.
fn write_unit(dir: &std::path::Path, path: &std::path::Path, text: &str) -> io::Result<bool> {
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
                return Ok(false);
            }
            // A unit Agora did not write is never replaced.
            if !was.starts_with(UNIT_MARKER.as_bytes()) {
                return Err(not_ours(path.to_path_buf()));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    // Written beside it (exclusive, 0600 from its creation) and renamed over it: `rename` replaces the name, never a link's target.
    crate::write_private(path, text.as_bytes())?;
    Ok(true)
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
    fn tricky_paths_are_quoted_for_systemd_without_a_shell() {
        let home = Path::new("/home/u/my \"q\" 'x' 50% $HOME ${X} ö\\ ş/");
        let mut s = spec(home);
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
            text.contains("WorkingDirectory=/home/u/my \"q\" 'x' 50%% $HOME ${X} ö\\ ş/\n"),
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
            "--user daemon-reload\n--user enable agora-node.service\n"
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
