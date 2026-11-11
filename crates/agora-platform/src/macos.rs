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
/// Where people's own files live. Reading is denied below these unless the caller lists the folder; the system folders stay readable,
/// because every program needs them to start at all (denying them aborts the process in dyld).
const PRIVATE: [&str; 4] = ["/Users", "/Volumes", "/private/var/folders", "/private/tmp"];

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

/// The profile for `sb`: writing is denied everywhere except below the `write` folders, and reading contents is denied below
/// [`PRIVATE`] except below the listed folders. Metadata stays readable so a path can be walked down to an allowed folder.
///
/// The allow names `file-read-data` itself: Seatbelt does not let `(allow file-read* ...)` undo an earlier
/// `(deny file-read-data ...)` (seen on macOS 27), so a wildcard allow would leave the agent unable to read its own worktree.
/// A listed folder that holds a [`PRIVATE`] folder (`/var` is `/private/var`, which holds every user's temp folder) is left out:
/// everything outside the private folders is readable already, and allowing the parent would open the private folder again.
///
/// A `write` entry that is a file also allows files next to it whose names start with its name and a dot (`.claude.json.lock`,
/// `.claude.json.tmp.123`): programs save a file by writing a sibling and renaming it, or lock it with one, and without them the
/// save fails while the file itself is writable. Nothing else in that folder opens up.
pub(crate) fn profile(sb: &crate::Sandbox) -> String {
    let real = |p: &std::path::Path| std::fs::canonicalize(p).ok();
    let holds_private = |p: &std::path::Path| {
        PRIVATE
            .iter()
            .map(std::path::Path::new)
            .any(|private| private != p && private.starts_with(p))
    };
    let reads: Vec<String> = sb
        .read
        .iter()
        .chain(sb.write.iter())
        .filter_map(|p| real(p))
        .filter(|p| !holds_private(p))
        .map(|p| quote(&p))
        .collect();
    let writes: Vec<String> = sb
        .write
        .iter()
        .filter_map(|p| real(p))
        .map(|p| quote(&p))
        .collect();
    let siblings: Vec<String> = sb
        .write
        .iter()
        .filter_map(|p| real(p))
        .filter(|p| p.is_file())
        .filter_map(|p| sibling_rule(&p))
        .collect();
    let private: Vec<String> = PRIVATE
        .iter()
        .map(|p| quote(std::path::Path::new(p)))
        .collect();
    let mut out = format!(
        "(version 1)\n(allow default)\n(deny file-write*)\n(deny file-read-data {})\n",
        private.join(" ")
    );
    if !reads.is_empty() {
        out.push_str(&format!("(allow file-read-data {})\n", reads.join(" ")));
    }
    if !writes.is_empty() {
        out.push_str(&format!("(allow file-write* {})\n", writes.join(" ")));
    }
    if !siblings.is_empty() {
        out.push_str(&format!(
            "(allow file-read-data file-write* {})\n",
            siblings.join(" ")
        ));
    }
    out
}

/// `(regex #"^<file>\.[^/]*$")`: the names next to `file` that start with its name and a dot. `None` for a path a raw string
/// cannot hold (a `"`), which then gets no siblings.
fn sibling_rule(file: &std::path::Path) -> Option<String> {
    let s = file.to_str()?;
    if s.contains('"') {
        return None;
    }
    let mut re = String::from("^");
    for c in s.chars() {
        if "\\.^$|?*+()[]{}".contains(c) {
            re.push('\\');
        }
        re.push(c);
    }
    re.push_str("\\.[^/]*$");
    Some(format!("(regex #\"{re}\")"))
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
        // The read allow names the denied operation; a wildcard does not undo a specific deny.
        let read = p.find("(allow file-read-data").unwrap();
        assert!(deny < read && !p.contains("(allow file-read*"), "{p}");
    }

    #[test]
    fn a_listed_file_lets_its_siblings_be_saved_and_nothing_else_in_its_folder() {
        let dir = std::env::temp_dir().join(format!("agora-sbx-sib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("state.json");
        std::fs::write(&file, "{}").unwrap();
        let sb = crate::Sandbox {
            read: vec!["/usr".into(), "/bin".into()],
            write: vec![file.clone(), "/dev".into()],
        };
        let p = super::profile(&sb);
        assert!(p.contains("state\\.json\\.[^/]*$"), "{p}");
        // Run it: the save pattern (sibling, then rename over the file) works; another name in the folder is refused.
        let script = format!(
            "cd {d} && echo new > state.json.tmp.1 && mv state.json.tmp.1 state.json && echo saved; echo x > other 2>/dev/null || echo refused",
            d = dir.display()
        );
        let out = std::process::Command::new(super::SANDBOX_EXEC)
            .args(["-p", &p, "/bin/sh", "-c", &script])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("saved") && text.contains("refused"), "{text}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new\n");
        assert!(!dir.join("other").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_listed_parent_of_a_private_folder_does_not_open_it() {
        let sb = crate::Sandbox {
            read: vec!["/var".into(), "/usr".into()],
            write: vec![],
        };
        let p = super::profile(&sb);
        // `/var` is `/private/var`, which holds `/private/var/folders`; it is left out, `/usr` stays.
        assert!(!p.contains("(subpath \"/private/var\")"), "{p}");
        assert!(
            p.contains("(allow file-read-data (subpath \"/usr\"))"),
            "{p}"
        );
    }
}
