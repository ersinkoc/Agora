//! Windows process view (ADR-0045): name, memory and CPU use, and the children of a process. There is no working directory: it
//! would mean reading another process's PEB, which is undocumented and changes between versions, so `process_cwd` stays unsupported
//! here and detection leans on the screen rules and on what agents report themselves.

use std::io;
use std::path::Path;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, SetHandleInformation, ERROR_ACCESS_DENIED, FILETIME, HANDLE,
    HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, STILL_ACTIVE,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess, GetProcessTimes,
    InitializeProcThreadAttributeList, OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
    UpdateProcThreadAttribute, CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP,
    CREATE_UNICODE_ENVIRONMENT, DETACHED_PROCESS, EXTENDED_STARTUPINFO_PRESENT,
    PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
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

/// The executable's file name without `.exe` (what the Unix backends call the short name).
pub(crate) fn name(pid: u32) -> io::Result<String> {
    let p = Process::open(pid)?;
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

pub(crate) fn children(pid: u32) -> io::Result<Vec<u32>> {
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
        if entry.th32ParentProcessID == pid && entry.th32ProcessID != pid {
            out.push(entry.th32ProcessID);
        }
        // SAFETY: as above.
        more = unsafe { Process32NextW(snap.0, &mut entry) } != 0;
    }
    Ok(out)
}

/// See [`crate::process_start_time`]: the creation time (100 ns units since 1601) of a process that has not ended.
pub(crate) fn start_time(pid: u32) -> Option<u64> {
    let p = Process::open(pid).ok()?;
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

/// See [`crate::spawn_detached`]: no console of its own, out of this console's Ctrl-C group and (when allowed) out of this process's
/// job. `CreateProcessW` directly, not `std::process::Command`: the standard library starts children with every inheritable handle of
/// this process, so a caller's output pipe would stay open until the daemon ends. Here only the two handles named in
/// `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` (the empty input and the log) are inherited.
pub(crate) fn spawn_detached(
    cmd: &std::process::Command,
    log: &std::fs::File,
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
        let base = DETACHED_PROCESS
            | CREATE_NEW_PROCESS_GROUP
            | EXTENDED_STARTUPINFO_PRESENT
            | CREATE_UNICODE_ENVIRONMENT;
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
            Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
                start(base).map(|pid| crate::Detached { pid, in_job: true })
            }
            Err(e) => Err(e),
        }
    };
    // SAFETY: initialised above; the child has its copy by now.
    unsafe { DeleteProcThreadAttributeList(list) };
    result
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

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
}
