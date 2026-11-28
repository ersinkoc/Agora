//! Windows process view (ADR-0045): name, memory and CPU use, and the children of a process. There is no working directory: it
//! would mean reading another process's PEB, which is undocumented and changes between versions, so `process_cwd` stays unsupported
//! here and detection leans on the screen rules and on what agents report themselves.

use std::io;
use std::path::Path;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, SetHandleInformation, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER,
    FILETIME, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, STILL_ACTIVE, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess, GetExitCodeProcess,
    GetProcessTimes, InitializeProcThreadAttributeList, OpenProcess, QueryFullProcessImageNameW,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject, CREATE_BREAKAWAY_FROM_JOB,
    CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, DETACHED_PROCESS,
    EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

fn last_error() -> io::Error {
    // SAFETY: plain call.
    io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
}

struct Process(HANDLE);

impl Process {
    fn open(pid: u32) -> io::Result<Self> {
        // SAFETY: plain call; the handle is closed on drop.
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if h.is_null() {
            return Err(last_error());
        }
        Ok(Self(h))
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: owned and closed once.
        unsafe { CloseHandle(self.0) };
    }
}

/// See [`crate::process_exists`]. A process that has ended stays queryable while any handle to it is open (the parent that started
/// it, a process viewer), so a lookup that succeeds says nothing; an ended process is a signalled object, so a wait of zero answers
/// it (Codex's review of #934). No: a pid that names nothing (`ERROR_INVALID_PARAMETER`), or a process that has ended. Yes: running,
/// or anything that cannot tell (access denied and an exit code of `STILL_ACTIVE`, an unexpected answer).
pub(crate) fn exists(pid: u32) -> bool {
    // SAFETY: plain call; the handle is closed on drop.
    let h = unsafe {
        OpenProcess(
            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        )
    };
    if !h.is_null() {
        let p = Process(h);
        // SAFETY: a live process handle opened with SYNCHRONIZE.
        return unsafe { WaitForSingleObject(p.0, 0) } != WAIT_OBJECT_0;
    }
    // SAFETY: plain call, right after the failed open.
    match unsafe { GetLastError() } {
        ERROR_INVALID_PARAMETER => false,
        // Some processes may be queried but not waited on: their exit code tells, unless it is the one that also means running.
        ERROR_ACCESS_DENIED => match Process::open(pid) {
            Ok(p) => {
                let mut code = 0u32;
                // SAFETY: a live handle opened with PROCESS_QUERY_LIMITED_INFORMATION and a valid out pointer.
                let ok = unsafe { GetExitCodeProcess(p.0, &mut code) } != 0;
                !ok || code == STILL_ACTIVE as u32
            }
            Err(e) => e.raw_os_error() != Some(ERROR_INVALID_PARAMETER as i32),
        },
        _ => true,
    }
}

/// The executable's file name without `.exe` (what the Unix backends call the short name).
pub(crate) fn name(pid: u32) -> io::Result<String> {
    name_of(&Process::open(pid)?)
}

/// [`name`] of an opened process.
fn name_of(p: &Process) -> io::Result<String> {
    let mut buf = vec![0u16; 32 * 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `buf` is valid for `len` UTF-16 units; flags 0 asks for the Win32 path.
    if unsafe { QueryFullProcessImageNameW(p.0, 0, buf.as_mut_ptr(), &mut len) } == 0 {
        return Err(last_error());
    }
    let full = String::from_utf16_lossy(&buf[..len as usize]);
    let file = Path::new(&full)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or(full);
    let lower = file.to_ascii_lowercase();
    Ok(if lower.ends_with(".exe") {
        file[..file.len() - 4].to_string()
    } else {
        file
    })
}

fn seconds(t: FILETIME) -> f64 {
    // 100 ns units.
    ((u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)) as f64 / 10_000_000.0
}

pub(crate) fn stats(pid: u32) -> io::Result<crate::ProcessStats> {
    let p = Process::open(pid)?;
    // SAFETY: all-zero FILETIMEs and counters are valid out-buffers.
    let (mut created, mut exited, mut kernel, mut user): (FILETIME, FILETIME, FILETIME, FILETIME) =
        unsafe { std::mem::zeroed() };
    // SAFETY: a live handle and valid out pointers.
    if unsafe { GetProcessTimes(p.0, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
        return Err(last_error());
    }
    let mut mem: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    mem.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: a live handle and a counters structure with its size set.
    if unsafe { GetProcessMemoryInfo(p.0, &mut mem, mem.cb) } == 0 {
        return Err(last_error());
    }
    Ok(crate::ProcessStats {
        rss_bytes: mem.WorkingSetSize as u64,
        cpu_seconds: seconds(kernel) + seconds(user),
    })
}

/// Whether a process listed with `pid` as its parent really is its child. Windows keeps the id of a parent that ended, and ids are
/// reused: a process created before `pid` was started by an earlier holder of that id, not by `pid`. When either creation time is
/// unknown (the process ended, or may not be opened), it is kept, as before.
fn started_after(parent_born: Option<u64>, child_born: Option<u64>) -> bool {
    !matches!((parent_born, child_born), (Some(p), Some(c)) if c < p)
}

pub(crate) fn children(pid: u32) -> io::Result<Vec<u32>> {
    let born = start_time(pid);
    // SAFETY: plain call.
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap == INVALID_HANDLE_VALUE {
        return Err(last_error());
    }
    let snap = Process(snap);
    // SAFETY: an all-zero entry with its size set is the documented starting state.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut out = Vec::new();
    // SAFETY: a live snapshot and an entry with its size set.
    let mut more = unsafe { Process32FirstW(snap.0, &mut entry) } != 0;
    while more {
        // A child whose id is reused between the snapshot and the time read is kept (its new holder is younger): rare and harmless.
        if entry.th32ParentProcessID == pid
            && entry.th32ProcessID != pid
            && started_after(born, start_time(entry.th32ProcessID))
        {
            out.push(entry.th32ProcessID);
        }
        // SAFETY: as above.
        more = unsafe { Process32NextW(snap.0, &mut entry) } != 0;
    }
    Ok(out)
}

/// See [`crate::process_start_time`]: the creation time (100 ns units since 1601) of a process that has not ended.
pub(crate) fn start_time(pid: u32) -> Option<u64> {
    started_of(&Process::open(pid).ok()?)
}

/// [`start_time`] of an opened process: `None` once it ended.
fn started_of(p: &Process) -> Option<u64> {
    let mut code = 0u32;
    // SAFETY: a live handle and a valid out pointer.
    if unsafe { GetExitCodeProcess(p.0, &mut code) } == 0 || code != STILL_ACTIVE as u32 {
        return None;
    }
    // SAFETY: all-zero FILETIMEs are valid out-buffers.
    let (mut created, mut exited, mut kernel, mut user): (FILETIME, FILETIME, FILETIME, FILETIME) =
        unsafe { std::mem::zeroed() };
    // SAFETY: a live handle and valid out pointers.
    if unsafe { GetProcessTimes(p.0, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
        return None;
    }
    Some((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

/// See [`crate::process_identity`]: the start time (100 ns units since 1601, wall-clock, so it already tells boots apart and `boot`
/// stays empty) and the program's name, both read from one handle, so a pid given to another process between two reads cannot mix
/// them. Windows has no process groups (`group` is 0).
pub(crate) fn identity(pid: u32) -> io::Result<crate::ProcessIdentity> {
    let p = Process::open(pid)?;
    let started = started_of(&p).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    Ok(crate::ProcessIdentity {
        pid,
        group: 0,
        started,
        boot: String::new(),
        name: name_of(&p)?,
    })
}

/// See [`crate::terminate_process`].
pub(crate) fn terminate(pid: u32) -> io::Result<()> {
    // SAFETY: plain call; the handle is closed on drop.
    let h = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
    if h.is_null() {
        return Err(last_error());
    }
    let p = Process(h);
    // SAFETY: a live handle opened with PROCESS_TERMINATE.
    if unsafe { TerminateProcess(p.0, 1) } == 0 {
        return Err(last_error());
    }
    Ok(())
}

/// What makes a process detached here: no console of its own and out of this console's Ctrl-C group.
pub(crate) const DETACHED_FLAGS: u32 = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;

/// A console program started with this gets a console with no window: nothing appears on screen and nothing takes the focus, and the
/// programs it starts in turn (git's hooks, `sh`, a credential helper) share that hidden console instead of each opening one.
pub(crate) const NO_WINDOW_FLAGS: u32 = CREATE_NO_WINDOW;

/// See [`crate::spawn_bound`]: puts this process, once, into a new job that ends every process in it when it is closed, and says
/// whether it is in one. The handle is never closed: this process holds the only one, so the job closes, and its processes end, exactly
/// when this process ends. Nested jobs (Windows 8 and later) let this work when this process is already in a job.
pub(crate) fn join_kill_on_close_job() -> bool {
    static JOINED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *JOINED.get_or_init(|| {
        // SAFETY: plain call.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return false;
        }
        // SAFETY: an all-zero limit structure means "no limits".
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
        // SAFETY: `info` is the structure the class names, with its size; live handles.
        let joined = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) != 0
                && AssignProcessToJobObject(job, GetCurrentProcess()) != 0
        };
        if !joined {
            // SAFETY: the handle created above, closed once.
            unsafe { CloseHandle(job) };
        }
        joined
    })
}

/// See [`crate::spawn_detached`]: no console of its own, out of this console's Ctrl-C group and (when allowed) out of this process's
/// job. `CreateProcessW` directly, not `std::process::Command`: the standard library starts children with every inheritable handle of
/// this process, so a caller's output pipe would stay open until the daemon ends. Here only the two handles named in
/// `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` (the empty input and the log) are inherited.
pub(crate) fn spawn_detached(
    cmd: &std::process::Command,
    log: &std::fs::File,
) -> io::Result<crate::Detached> {
    spawn_detached_with(cmd, log, true)
}

/// ADR-0079 decision 2: [`spawn_detached`] without its fallback. A process that must outlive this one's job (the terminal holder) is
/// never started inside it: when the job forbids leaving it (a Task Scheduler task's own job), nothing starts and the error is
/// `PermissionDenied` (`ERROR_ACCESS_DENIED`).
pub(crate) fn spawn_detached_strict(
    cmd: &std::process::Command,
    log: &std::fs::File,
) -> io::Result<crate::Detached> {
    spawn_detached_with(cmd, log, false)
}

/// The body of [`spawn_detached`] and [`spawn_detached_strict`]: `fallback` says whether a refused breakaway starts the process inside
/// the job instead.
fn spawn_detached_with(
    cmd: &std::process::Command,
    log: &std::fs::File,
    fallback: bool,
) -> io::Result<crate::Detached> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use std::ptr::{null, null_mut};

    let text = |s: &std::ffi::OsStr| {
        s.to_str()
            .map(crate::conpty::quote_arg)
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))
    };
    let mut line = vec![text(cmd.get_program())?];
    for a in cmd.get_args() {
        line.push(text(a)?);
    }
    let mut cmdline: Vec<u16> = line
        .join(" ")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let program: Vec<u16> = cmd
        .get_program()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // This process's environment with the command's changes (names compare without case, as Windows does).
    let mut vars: Vec<(std::ffi::OsString, std::ffi::OsString)> = std::env::vars_os().collect();
    for (k, v) in cmd.get_envs() {
        let same = |have: &std::ffi::OsString| {
            have.to_string_lossy()
                .eq_ignore_ascii_case(&k.to_string_lossy())
        };
        vars.retain(|(have, _)| !same(have));
        if let Some(v) = v {
            vars.push((k.to_os_string(), v.to_os_string()));
        }
    }
    let mut env: Vec<u16> = Vec::new();
    for (k, v) in vars {
        env.extend(k.encode_wide());
        env.push(u16::from(b'='));
        env.extend(v.encode_wide());
        env.push(0);
    }
    env.push(0);
    let cwd: Option<Vec<u16>> = cmd.get_current_dir().map(|d| {
        d.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    });

    // The two handles the child gets, made inheritable (only they are listed, so nothing else of ours goes with them).
    let nul = std::fs::OpenOptions::new().read(true).open("NUL")?;
    let handles: [HANDLE; 2] = [nul.as_raw_handle() as HANDLE, log.as_raw_handle() as HANDLE];
    for h in handles {
        // SAFETY: live handles owned by the files above.
        if unsafe { SetHandleInformation(h, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
            return Err(last_error());
        }
    }

    let mut size = 0usize;
    // SAFETY: the documented size query: null list, one attribute.
    unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut size) };
    let mut storage = vec![0u8; size];
    let list = storage.as_mut_ptr().cast::<c_void>();
    // SAFETY: `storage` is `size` bytes and outlives the list.
    if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) } == 0 {
        return Err(last_error());
    }
    // SAFETY: `handles` outlives the list and the call; its size is given in bytes.
    let ok = unsafe {
        UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            handles.as_ptr().cast(),
            std::mem::size_of_val(&handles),
            null_mut(),
            null(),
        )
    };
    let result = if ok == 0 {
        Err(last_error())
    } else {
        // SAFETY: an all-zero STARTUPINFOEXW is the documented starting point.
        let mut si: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
        si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        si.lpAttributeList = list;
        si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        si.StartupInfo.hStdInput = handles[0];
        si.StartupInfo.hStdOutput = handles[1];
        si.StartupInfo.hStdError = handles[1];
        let base = DETACHED_FLAGS | EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT;
        let mut start = |flags: u32| -> io::Result<u32> {
            // SAFETY: an all-zero PROCESS_INFORMATION is a valid out-buffer.
            let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
            // SAFETY: every pointer is valid for the call; `cmdline` is mutable as the API requires; handles are inherited (TRUE) but
            // only those in the attribute list.
            let created = unsafe {
                CreateProcessW(
                    program.as_ptr(),
                    cmdline.as_mut_ptr(),
                    null(),
                    null(),
                    1,
                    flags,
                    env.as_ptr().cast(),
                    cwd.as_ref().map_or(null(), |c| c.as_ptr()),
                    &si.StartupInfo,
                    &mut pi,
                )
            };
            if created == 0 {
                return Err(last_error());
            }
            drop(Process(pi.hThread));
            drop(Process(pi.hProcess));
            Ok(pi.dwProcessId)
        };
        match start(base | CREATE_BREAKAWAY_FROM_JOB) {
            Ok(pid) => Ok(crate::Detached { pid, in_job: false }),
            // A job without JOB_OBJECT_LIMIT_BREAKAWAY_OK refuses with "access denied": start inside it, and say so.
            Err(e) if fallback && e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
                start(base).map(|pid| crate::Detached { pid, in_job: true })
            }
            Err(e) => Err(e),
        }
    };
    // SAFETY: initialised above; the child has its copy by now.
    unsafe { DeleteProcThreadAttributeList(list) };
    result
}

/// One process of a Toolhelp snapshot, as ADR-0094's lookup needs it: its id, its parent's id and its image name in lower case without
/// `.exe`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    pid: u32,
    parent: u32,
    name: Box<str>,
}

/// Programs the lookup goes through to the program a person started in them (ADR-0094 Amendment A): shells, and `cmd.exe`, which also
/// runs a `.cmd` shim.
const SHELLS: [&str; 5] = ["cmd", "pwsh", "powershell", "sh", "bash"];

/// How deep the lookup follows shells started in shells.
const MAX_SHELL_DEPTH: usize = 8;

/// How long one snapshot answers every terminal's lookup: the node asks for each watched or busy shell once per 2 s tick, and one
/// snapshot of the machine costs 15 to 30 ms (ADR-0094 Amendment A), so the shells of one tick share it.
const SHARE_SNAPSHOT_FOR: std::time::Duration = std::time::Duration::from_secs(1);

/// ADR-0094 Amendment A: the program in front of the pseudoconsole whose shell is `shell`. That is the shell's newest direct child (by
/// creation time, and created after the shell, so a reused parent id is not followed), followed down while that child is itself a
/// shell. `shell` itself when nothing runs in it. Windows has no foreground group: a program started in the background is a child too.
/// Image names only; arguments are never read.
pub(crate) fn foreground(shell: u32) -> io::Result<u32> {
    let tree = shared_snapshot()?;
    Ok(front_of(&tree, shell, &start_time))
}

/// The rule of [`foreground`] on a given tree; `born` gives a process's creation time, `None` once it has ended.
fn front_of(tree: &[Entry], shell: u32, born: &dyn Fn(u32) -> Option<u64>) -> u32 {
    let mut at = shell;
    let mut at_born = born(shell);
    for _ in 0..MAX_SHELL_DEPTH {
        let newest = tree
            .iter()
            .filter(|e| e.parent == at && e.pid != at)
            .filter_map(|e| born(e.pid).map(|b| (b, e)))
            .filter(|(b, _)| started_after(at_born, Some(*b)))
            .max_by_key(|(b, e)| (*b, e.pid));
        let Some((b, child)) = newest else {
            return at;
        };
        if !SHELLS.contains(&&*child.name) {
            return child.pid;
        }
        at = child.pid;
        at_born = Some(b);
    }
    at
}

/// A Toolhelp snapshot of the machine, taken at most once per [`SHARE_SNAPSHOT_FOR`] and shared by every caller meanwhile.
fn shared_snapshot() -> io::Result<std::sync::Arc<[Entry]>> {
    use std::sync::{Arc, Mutex};
    use std::time::Instant;
    static SHARED: Mutex<Option<(Instant, Arc<[Entry]>)>> = Mutex::new(None);
    // Held while a new snapshot is taken, so callers of one tick wait for that one instead of taking their own.
    let mut shared = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, tree)) = shared.as_ref() {
        if at.elapsed() < SHARE_SNAPSHOT_FOR {
            return Ok(Arc::clone(tree));
        }
    }
    let tree: Arc<[Entry]> = snapshot()?.into();
    *shared = Some((Instant::now(), Arc::clone(&tree)));
    Ok(tree)
}

fn snapshot() -> io::Result<Vec<Entry>> {
    // SAFETY: plain call.
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap == INVALID_HANDLE_VALUE {
        return Err(last_error());
    }
    let snap = Process(snap);
    // SAFETY: an all-zero entry with its size set is the documented starting state.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut out = Vec::with_capacity(1024);
    // SAFETY: a live snapshot and an entry with its size set.
    let mut more = unsafe { Process32FirstW(snap.0, &mut entry) } != 0;
    while more {
        let file = &entry.szExeFile;
        let len = file.iter().position(|&c| c == 0).unwrap_or(file.len());
        let mut name = String::from_utf16_lossy(&file[..len]).to_ascii_lowercase();
        if name.ends_with(".exe") {
            name.truncate(name.len() - 4);
        }
        out.push(Entry {
            pid: entry.th32ProcessID,
            parent: entry.th32ParentProcessID,
            name: name.into(),
        });
        // SAFETY: as above.
        more = unsafe { Process32NextW(snap.0, &mut entry) } != 0;
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn a_process_older_than_its_listed_parent_is_not_its_child() {
        // A parent id reused: the "child" was created before this parent.
        assert!(!started_after(Some(200), Some(100)));
        assert!(started_after(Some(100), Some(200)));
        assert!(started_after(Some(100), Some(100)));
        // Unknown times keep the child, as before.
        assert!(started_after(None, Some(1)));
        assert!(started_after(Some(1), None));
        assert!(started_after(None, None));
    }

    /// A fake tree for ADR-0094's rule: `(pid, parent, name, born)`; a `None` birth is a process that has ended.
    fn tree(rows: &[(u32, u32, &str, Option<u64>)]) -> (Vec<Entry>, impl Fn(u32) -> Option<u64>) {
        let entries = rows
            .iter()
            .map(|&(pid, parent, name, _)| Entry {
                pid,
                parent,
                name: name.into(),
            })
            .collect();
        let births: std::collections::HashMap<u32, Option<u64>> =
            rows.iter().map(|&(pid, _, _, b)| (pid, b)).collect();
        (entries, move |pid| births.get(&pid).copied().flatten())
    }

    #[test]
    fn the_shell_is_in_front_when_nothing_runs_in_it() {
        let (t, born) = tree(&[(10, 1, "cmd", Some(100)), (11, 10, "cmd", None)]);
        assert_eq!(front_of(&t, 10, &born), 10);
    }

    /// Amendment A: Claude Code starts helpers below itself; they are newer, but never in front.
    #[test]
    fn the_helpers_a_program_starts_never_win() {
        let (t, born) = tree(&[
            (10, 1, "cmd", Some(100)),
            (20, 10, "claude", Some(200)),
            (21, 20, "conhost", Some(300)),
            (22, 20, "reg", Some(301)),
            (23, 20, "git", Some(302)),
            (24, 23, "conhost", Some(303)),
        ]);
        assert_eq!(front_of(&t, 10, &born), 20);
    }

    /// Codex installed with bun: the shim, then node, then the real binary. The shim is what the shell started, and it is named codex.
    #[test]
    fn a_bun_shim_is_the_program_in_front() {
        let (t, born) = tree(&[
            (10, 1, "pwsh", Some(100)),
            (20, 10, "codex", Some(200)),
            (21, 20, "node", Some(201)),
            (22, 21, "codex", Some(202)),
        ]);
        assert_eq!(front_of(&t, 10, &born), 20);
    }

    #[test]
    fn a_shell_started_in_the_shell_is_followed_down() {
        let (t, born) = tree(&[
            (10, 1, "cmd", Some(100)),
            (20, 10, "pwsh", Some(200)),
            (30, 20, "claude", Some(300)),
            (31, 30, "conhost", Some(400)),
        ]);
        assert_eq!(front_of(&t, 10, &born), 30);
        // With nothing in the inner shell, the inner shell is in front (no manifest names a shell).
        let (t, born) = tree(&[(10, 1, "cmd", Some(100)), (20, 10, "pwsh", Some(200))]);
        assert_eq!(front_of(&t, 10, &born), 20);
    }

    /// An npm-style `.cmd` shim run from pwsh: cmd.exe is followed, and the program is node, which no manifest names (no mark).
    #[test]
    fn a_cmd_shim_ends_at_node() {
        let (t, born) = tree(&[
            (10, 1, "pwsh", Some(100)),
            (20, 10, "cmd", Some(200)),
            (30, 20, "node", Some(300)),
        ]);
        assert_eq!(front_of(&t, 10, &born), 30);
    }

    /// The newest direct child wins (a program started in the background earlier loses to the one started after it); one that ended,
    /// and one older than the shell that holds a reused parent id, are not children.
    #[test]
    fn only_the_newest_live_child_started_after_the_shell_counts() {
        let (t, born) = tree(&[
            (10, 1, "cmd", Some(100)),
            (20, 10, "codex", Some(200)),
            (21, 10, "claude", Some(300)),
            (22, 10, "wstack", None),
            (23, 10, "old", Some(50)),
        ]);
        assert_eq!(front_of(&t, 10, &born), 21);
        let (t, born) = tree(&[(10, 1, "cmd", Some(100)), (23, 10, "old", Some(50))]);
        assert_eq!(front_of(&t, 10, &born), 10);
    }

    #[test]
    fn a_chain_of_shells_stops_at_the_depth_limit() {
        let rows: Vec<(u32, u32, &str, Option<u64>)> = (0..20u32)
            .map(|i| {
                (
                    100 + i,
                    if i == 0 { 1 } else { 99 + i },
                    "cmd",
                    Some(u64::from(i)),
                )
            })
            .collect();
        let (t, born) = tree(&rows);
        assert_eq!(front_of(&t, 100, &born), 100 + MAX_SHELL_DEPTH as u32);
    }

    /// Amendment A on a real machine: a snapshot lists this process under its own parent with its name.
    #[test]
    fn a_snapshot_lists_this_process_with_its_parent_and_name() {
        let me = std::process::id();
        let t = snapshot().unwrap();
        let mine = t.iter().find(|e| e.pid == me).unwrap();
        assert_eq!(*mine.name, name(me).unwrap().to_ascii_lowercase());
        assert_ne!(mine.parent, 0);
        // Shared: lookups right after one another read one snapshot (a second try covers a window that ended between the first two).
        let shared = (0..2).any(|_| {
            let a = shared_snapshot().unwrap();
            let b = shared_snapshot().unwrap();
            std::sync::Arc::ptr_eq(&a, &b)
        });
        assert!(shared);
    }

    #[test]
    fn this_process_has_a_name_memory_and_cpu_time() {
        let me = std::process::id();
        let n = name(me).unwrap();
        assert!(
            !n.is_empty() && !n.to_ascii_lowercase().ends_with(".exe"),
            "{n}"
        );
        let a = stats(me).unwrap();
        assert!(a.rss_bytes > 100_000, "{a:?}");
        let mut x = 0u64;
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(300) {
            x = x.wrapping_mul(31).wrapping_add(1);
        }
        std::hint::black_box(x);
        assert!(stats(me).unwrap().cpu_seconds >= a.cpu_seconds);
        assert!(stats(u32::MAX - 1).is_err());
    }

    #[test]
    fn a_child_is_found_under_its_parent_and_in_the_tree() {
        let mut child = std::process::Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let me = std::process::id();
        assert!(children(me).unwrap().contains(&child.id()));
        let tree = crate::process_tree(me).unwrap();
        assert_eq!(tree[0].pid, me);
        assert!(tree
            .iter()
            .any(|p| p.pid == child.id() && p.ppid == me && p.name.eq_ignore_ascii_case("ping")));
        assert!(crate::process_cwd(me).is_err());
        let _ = child.kill();
        let _ = child.wait();
    }

    /// A helper the node starts has a console but no window: nothing flashes on screen or takes the focus (the maintainer saw one
    /// window per `git` while looking at a diff). The parent must have no console, as the node has none: this test starts its own
    /// binary detached and that copy runs [`no_window_probe`], which starts the probe both ways. Without the flag the probe gets a
    /// window of its own (the bug, so the check can tell); with it, none.
    #[test]
    fn a_helper_started_with_no_console_window_has_no_window() {
        let exe = std::env::current_exe().expect("test binary");
        let mut cmd = std::process::Command::new(exe);
        cmd.args([
            "--exact",
            "winproc::tests::no_window_probe",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("AGORA_NO_WINDOW_PROBE", "1");
        std::os::windows::process::CommandExt::creation_flags(&mut cmd, DETACHED_FLAGS);
        let out = cmd.output().expect("detached test binary");
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text
            .lines()
            .find_map(|l| l.split_once("PROBE ").map(|(_, rest)| rest))
            .unwrap_or_else(|| panic!("no probe line: {text}"));
        let (with, without) = line.split_once(' ').expect("two handles");
        assert_eq!(with, "with=0", "{line}");
        assert_ne!(without, "without=0", "{line}");
    }

    /// The probe's console window handle, started with or without [`crate::no_console_window`].
    fn console_window(hidden: bool) -> String {
        let mut cmd = std::process::Command::new("powershell.exe");
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$w = Add-Type -Name W -Namespace NoWindow -PassThru -MemberDefinition \
             '[DllImport(\"kernel32.dll\")] public static extern System.IntPtr GetConsoleWindow();'; \
             [Console]::Out.Write([long]$w::GetConsoleWindow())",
        ]);
        if hidden {
            crate::no_console_window(&mut cmd);
        }
        let out = cmd.output().expect("powershell");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Runs only inside [`a_helper_started_with_no_console_window_has_no_window`]'s detached copy.
    #[test]
    fn no_window_probe() {
        if std::env::var_os("AGORA_NO_WINDOW_PROBE").is_none() {
            return;
        }
        let with = console_window(true);
        let without = console_window(false);
        println!("PROBE with={with} without={without}");
    }
}

/// M2.6: a process's identity on Windows, as `unix.rs`'s identity tests check it on Unix.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod identity_tests {
    use crate::{kill_process_if_same, process_identity, process_still_running};

    #[test]
    fn a_process_is_known_by_its_start_and_a_reused_pid_is_never_killed() {
        let mut child = std::process::Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let id = process_identity(child.id()).unwrap();
        assert_eq!(id.name.to_ascii_lowercase(), "ping");
        assert_eq!((id.group, id.boot.as_str()), (0, ""));
        assert_eq!(Some(id.started), crate::process_start_time(child.id()));
        assert!(process_still_running(&id));
        // Another start time, another boot, another program, or no program named: another process, left alone.
        let others = [
            crate::ProcessIdentity {
                started: id.started + 1,
                ..id.clone()
            },
            crate::ProcessIdentity {
                boot: "x".into(),
                ..id.clone()
            },
            crate::ProcessIdentity {
                name: "victim".into(),
                ..id.clone()
            },
            crate::ProcessIdentity {
                name: String::new(),
                ..id.clone()
            },
        ];
        for other in &others {
            assert!(!process_still_running(other), "{other:?}");
            assert!(!kill_process_if_same(other).unwrap(), "{other:?}");
        }
        assert!(child.try_wait().unwrap().is_none());
        assert!(kill_process_if_same(&id).unwrap());
        assert!(!child.wait().unwrap().success());
        // Ended: nothing to read, and nothing to kill.
        assert!(process_identity(id.pid).is_err() || !process_still_running(&id));
        assert!(!kill_process_if_same(&id).unwrap());
    }
}
