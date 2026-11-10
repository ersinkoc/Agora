//! Process inspection through `libproc`.

use std::ffi::CStr;
use std::io;
use std::path::PathBuf;

/// Mach absolute-time to nanoseconds ratio as `(numer, denom)`.
#[allow(deprecated)] // `libc` prefers the `mach2` crate; one stable call is not worth a new dependency
fn timebase() -> (u64, u64) {
    // SAFETY: an all-zero mach_timebase_info is a valid out-buffer.
    let mut tb: libc::mach_timebase_info = unsafe { std::mem::zeroed() };
    // SAFETY: valid out-pointer.
    unsafe { libc::mach_timebase_info(&mut tb) };
    if tb.denom == 0 {
        (1, 1)
    } else {
        (u64::from(tb.numer), u64::from(tb.denom))
    }
}

pub(crate) fn stats(pid: u32) -> io::Result<crate::ProcessStats> {
    // SAFETY: an all-zero proc_taskinfo is a valid buffer for the kernel to fill.
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskinfo>() as i32;
    // SAFETY: the buffer is a live proc_taskinfo of `size` bytes.
    let n = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTASKINFO,
            0,
            (&mut info as *mut libc::proc_taskinfo).cast(),
            size,
        )
    };
    if n <= 0 {
        return Err(io::Error::last_os_error());
    }
    // CPU times are in mach absolute-time units: nanoseconds on Intel, a different tick on Apple silicon.
    let (numer, denom) = timebase();
    let ns = (info.pti_total_user + info.pti_total_system) as f64 * numer as f64 / denom as f64;
    Ok(crate::ProcessStats {
        rss_bytes: info.pti_resident_size,
        cpu_seconds: ns / 1e9,
    })
}

pub(crate) fn name(pid: u32) -> io::Result<String> {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for `buf.len()` bytes.
    let n = unsafe { libc::proc_name(pid as i32, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n <= 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

pub(crate) fn cwd(pid: u32) -> io::Result<PathBuf> {
    // SAFETY: an all-zero proc_vnodepathinfo is a valid (empty) value for the kernel to fill.
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as i32;
    // SAFETY: the buffer is a live proc_vnodepathinfo of `size` bytes.
    let n = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            (&mut info as *mut libc::proc_vnodepathinfo).cast(),
            size,
        )
    };
    if n <= 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the kernel NUL-terminates the path inside the fixed-size array.
    let path = unsafe { CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr().cast()) };
    Ok(PathBuf::from(path.to_string_lossy().into_owned()))
}

pub(crate) fn children(pid: u32) -> io::Result<Vec<u32>> {
    let mut buf = vec![0i32; 1024];
    loop {
        buf.iter_mut().for_each(|p| *p = 0);
        // SAFETY: the buffer is valid for the byte size passed.
        let r = unsafe {
            libc::proc_listchildpids(
                pid as i32,
                buf.as_mut_ptr().cast(),
                (buf.len() * std::mem::size_of::<i32>()) as i32,
            )
        };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        // Whether the return value counts bytes or pids differs between libproc versions, so read the
        // zero-initialised buffer instead: unused slots stay 0. A full buffer may be truncated, so grow it.
        if buf.last().is_some_and(|p| *p != 0) {
            buf = vec![0i32; buf.len() * 2];
            continue;
        }
        let mut out: Vec<u32> = buf.iter().filter(|p| **p > 0).map(|p| *p as u32).collect();
        out.sort_unstable();
        return Ok(out);
    }
}

// ---- Seatbelt (ADR-0047): the profile language `sandbox-exec` enforces on a process and everything it starts ----

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
/// What every program needs to start at all; the caller's lists come on top of these.
const SYSTEM_READ: [&str; 8] = [
    "/System",
    "/Library",
    "/usr",
    "/bin",
    "/sbin",
    "/private/etc",
    "/private/var/db",
    "/dev",
];

/// True when `sandbox-exec` is on this machine.
pub(crate) fn sandbox_supported() -> bool {
    std::path::Path::new(SANDBOX_EXEC).exists()
}

fn quote(p: &std::path::Path) -> String {
    let s = p
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("(subpath \"{s}\")")
}

/// The profile for `sb`: everything is allowed except reading contents and writing files, which are denied and then allowed again
/// below the listed folders. Reading a file's metadata stays open so a path can be walked down to an allowed folder.
pub(crate) fn profile(sb: &crate::Sandbox) -> String {
    let real = |p: &std::path::Path| std::fs::canonicalize(p).ok();
    let reads: Vec<String> = SYSTEM_READ
        .iter()
        .map(std::path::PathBuf::from)
        .chain(sb.read.iter().cloned())
        .chain(sb.write.iter().cloned())
        .filter_map(|p| real(&p))
        .map(|p| quote(&p))
        .collect();
    let writes: Vec<String> = sb
        .write
        .iter()
        .filter_map(|p| real(p))
        .map(|p| quote(&p))
        .collect();
    let mut out = String::from(
        "(version 1)\n(allow default)\n(deny file-read-data file-write*)\n(allow file-read-metadata)\n",
    );
    if !reads.is_empty() {
        out.push_str(&format!("(allow file-read* {})\n", reads.join(" ")));
    }
    if !writes.is_empty() {
        out.push_str(&format!("(allow file-write* {})\n", writes.join(" ")));
    }
    out
}

/// `program args...` rewritten to run under `sandbox-exec` with the profile for `sb`.
pub(crate) fn wrap(
    sb: &crate::Sandbox,
    program: &std::ffi::OsStr,
    args: &[std::ffi::OsString],
) -> (std::ffi::OsString, Vec<std::ffi::OsString>) {
    let mut v: Vec<std::ffi::OsString> =
        vec!["-p".into(), profile(sb).into(), "--".into(), program.into()];
    v.extend(args.iter().cloned());
    (SANDBOX_EXEC.into(), v)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod sandbox_tests {
    #[test]
    fn the_profile_denies_first_and_allows_listed_folders_after() {
        let sb = crate::Sandbox {
            read: vec!["/usr".into()],
            write: vec![std::env::temp_dir()],
        };
        let p = super::profile(&sb);
        let deny = p.find("(deny file-read-data").unwrap();
        let allow = p.find("(allow file-write*").unwrap();
        assert!(deny < allow, "{p}");
    }
}
