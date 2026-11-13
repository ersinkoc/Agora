//! Windows process view (ADR-0045): name, memory and CPU use, and the children of a process. There is no working directory: it
//! would mean reading another process's PEB, which is undocumented and changes between versions, so `process_cwd` stays unsupported
//! here and detection leans on the screen rules and on what agents report themselves.

use std::io;
use std::path::Path;

use windows_sys::Win32::Foundation::STILL_ACTIVE;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, FILETIME, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
    CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_TERMINATE,
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

/// See [`crate::spawn_detached`]: no console of its own and out of this console's Ctrl-C group.
pub(crate) fn detach(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
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
