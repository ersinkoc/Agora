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

/// `(literal "p")`: that path itself, not what is below it.
fn literal(p: &std::path::Path) -> String {
    let s = p
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("(literal \"{s}\")")
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
///
/// Fails when a `deny` entry is relative or cannot be resolved: a closed folder that silently fell out of the profile would leave the
/// node's home open.
pub(crate) fn profile(sb: &crate::Sandbox) -> io::Result<String> {
    let mut denied: Vec<std::path::PathBuf> = Vec::new();
    for p in &sb.deny {
        match real_or_future(p).filter(|_| p.is_absolute()) {
            Some(r) => denied.push(r),
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a protected path cannot be resolved",
                ))
            }
        }
    }
    // A `read` or `write` entry that resolves below a closed folder (`~/.claude` made a link to the node's home) is left out: the
    // closed folder stays closed whatever the agent left behind for its next start.
    let real = |p: &std::path::Path| {
        std::fs::canonicalize(p)
            .ok()
            .filter(|r| !denied.iter().any(|d| r.starts_with(d)))
    };
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
    // The node's own folders, closed after everything opened above: in Seatbelt the last matching rule wins, so they stay closed below
    // an allowed folder. A unix socket is reached by `network-outbound`, which file rules do not cover (measured: a confined process
    // connected to a socket below a folder it could not read), so its sockets are closed too, then the agent's own reopened.
    // A protected path that does not exist yet (the control socket while interrupted agents resume, before the control server
    // starts) is closed all the same: the rule names where it will be, through its nearest existing parent.
    if !denied.is_empty() {
        let paths: Vec<String> = denied.iter().map(|p| quote(p)).collect();
        let to: Vec<String> = denied
            .iter()
            .map(|p| format!("(remote unix-socket {})", quote(p)))
            .collect();
        let bind: Vec<String> = denied
            .iter()
            .map(|p| format!("(local unix-socket {})", quote(p)))
            .collect();
        // Everything below them: contents, metadata and extended attributes (their names and sizes say what is there), writes,
        // and connecting to or creating a socket there (an agent resumed before the node binds its control socket must not bind
        // it first).
        // Each read named: a wildcard `file-read*` deny does not undo an earlier `allow file-read-data` (measured: the data stayed
        // readable below an allowed `/tmp`), as a wildcard allow does not undo a named deny.
        out.push_str(&format!(
            "(deny file-read-data file-read-metadata file-read-xattr file-write* {})\n",
            paths.join(" ")
        ));
        out.push_str(&format!("(deny network-outbound {})\n", to.join(" ")));
        out.push_str(&format!("(deny network-bind {})\n", bind.join(" ")));
        // A closed folder cannot be moved or removed by moving a folder above it: below a folder the agent may write (a node home
        // under `/tmp`), `mv /tmp/home /tmp/x` would carry everything out from under rules that name its path. The folders on the
        // way to it are closed to writes themselves (their own entry: renaming or removing them), not to what is created in them.
        let writable: Vec<std::path::PathBuf> = sb.write.iter().filter_map(|p| real(p)).collect();
        let mut above: Vec<String> = Vec::new();
        for d in &denied {
            for a in d.ancestors().skip(1) {
                if a.parent().is_some() && writable.iter().any(|w| a.starts_with(w)) {
                    let q = literal(a);
                    if !above.contains(&q) {
                        above.push(q);
                    }
                }
            }
        }
        if !above.is_empty() {
            out.push_str(&format!("(deny file-write* {})\n", above.join(" ")));
        }
        // What the agent may use below a closed folder is opened again after it: its own worktree (an allow-list: everything else
        // of the node's home stays closed), with the metadata of the folders on the way, so a path to it can still be walked
        // (`git` resolves every folder of its working directory). Only while it is still a real folder: the agent could have
        // replaced it with a link to the home, which would then be opened as its worktree.
        let inside: Vec<std::path::PathBuf> = sb
            .reopen
            .iter()
            .filter(|p| p.is_absolute())
            .filter(|p| {
                std::fs::symlink_metadata(p)
                    .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
            })
            .filter_map(|p| std::fs::canonicalize(p).ok())
            .filter(|w| denied.iter().any(|d| w.starts_with(d) && w != d))
            .collect();
        if !inside.is_empty() {
            let open: Vec<String> = inside.iter().map(|w| quote(w)).collect();
            out.push_str(&format!(
                "(allow file-read-data file-read-metadata file-read-xattr file-write* {})\n",
                open.join(" ")
            ));
            let mut walk: Vec<String> = Vec::new();
            for w in &inside {
                for a in w.ancestors().skip(1) {
                    if denied.iter().any(|d| a.starts_with(d)) {
                        let q = literal(a);
                        if !walk.contains(&q) {
                            walk.push(q);
                        }
                    }
                }
            }
            if !walk.is_empty() {
                out.push_str(&format!("(allow file-read-metadata {})\n", walk.join(" ")));
            }
        }
        let own: Vec<String> = sb
            .sockets
            .iter()
            .filter_map(|p| real_or_new(p))
            .map(|p| format!("(remote unix-socket {})", quote(&p)))
            .collect();
        if !own.is_empty() && !sb.offline {
            out.push_str(&format!("(allow network-outbound {})\n", own.join(" ")));
        }
    }
    if sb.offline {
        // ADR-0047 `worktree-offline`: nothing leaves the machine. Seatbelt names only `localhost` (this machine's own addresses) or
        // every host. Connections to this machine stay open (a local model; see the threat model for what else listens there);
        // listening on a TCP port does not, because Seatbelt cannot tell a loopback-only listener from one on every interface
        // (measured: an `inbound localhost` rule let a LAN peer in through `0.0.0.0`). Unix sockets only below `sockets` (the agent's
        // own mesh socket and private temp folder): `/tmp` holds a tmux server's and an SSH agent's sockets, either a way out.
        out.push_str("(deny network*)\n(allow network-outbound (remote ip \"localhost:*\"))\n");
        let sockets: Vec<String> = sb
            .sockets
            .iter()
            .filter_map(|p| real_or_new(p))
            .map(|p| quote(&p))
            .collect();
        if !sockets.is_empty() {
            let to: Vec<String> = sockets
                .iter()
                .map(|q| format!("(remote unix-socket {q})"))
                .collect();
            let own: Vec<String> = sockets
                .iter()
                .map(|q| format!("(local unix-socket {q})"))
                .collect();
            out.push_str(&format!("(allow network-outbound {})\n", to.join(" ")));
            out.push_str(&format!("(allow network-bind {})\n", own.join(" ")));
        }
    }
    Ok(out)
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

/// The canonical form of `p`, or, for a path that does not exist yet (a socket made later), its canonical parent joined with its name.
/// `p` resolved as far as it exists (symbolic links in its existing part followed: `/tmp` is `/private/tmp`), the rest of it as
/// written. `None` only when not even its root resolves.
fn real_or_future(p: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut missing = Vec::new();
    let mut at = p;
    loop {
        if let Ok(real) = std::fs::canonicalize(at) {
            return Some(missing.iter().rev().fold(real, |acc, part| acc.join(part)));
        }
        missing.push(at.file_name()?.to_os_string());
        at = at.parent()?;
    }
}

fn real_or_new(p: &std::path::Path) -> Option<std::path::PathBuf> {
    std::fs::canonicalize(p).ok().or_else(|| {
        Some(
            std::fs::canonicalize(p.parent()?)
                .ok()?
                .join(p.file_name()?),
        )
    })
}

/// `program args...` rewritten to run under `sandbox-exec` with the profile for `sb`.
pub(crate) fn wrap(
    sb: &crate::Sandbox,
    program: &std::ffi::OsStr,
    args: &[std::ffi::OsString],
) -> io::Result<(std::ffi::OsString, Vec<std::ffi::OsString>)> {
    let mut v: Vec<std::ffi::OsString> = vec![
        "-p".into(),
        profile(sb)?.into(),
        "--".into(),
        program.into(),
    ];
    v.extend(args.iter().cloned());
    Ok((SANDBOX_EXEC.into(), v))
}

/// See [`crate::opened_path`]: `F_GETPATH`, the path the kernel has for the open vnode (firmlinks shown as `/Users/...`, as
/// `canonicalize` shows them).
pub(crate) fn opened_path(file: &std::fs::File) -> io::Result<PathBuf> {
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    let mut buf = vec![0u8; libc::PATH_MAX as usize];
    // SAFETY: F_GETPATH writes a NUL-terminated path of at most PATH_MAX bytes into `buf`, which holds that many.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buf.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    let len = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(&buf[..len])))
}

/// Process group and start time (microseconds since the epoch, `proc_bsdinfo`) of a running process.
pub(crate) fn identity(pid: u32) -> io::Result<crate::ProcessIdentity> {
    // SAFETY: an all-zero proc_bsdinfo is a valid buffer for the kernel to fill.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    // SAFETY: the buffer is a live proc_bsdinfo of `size` bytes.
    let n = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    if n < size {
        return Err(io::Error::last_os_error());
    }
    // SZOMB = 5: a process that ended and waits to be reaped is not running.
    if info.pbi_status == 5 {
        return Err(io::Error::new(io::ErrorKind::NotFound, "zombie"));
    }
    Ok(crate::ProcessIdentity {
        pid,
        group: info.pbi_pgid,
        started: info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
        // The start time is wall-clock time to the microsecond: another boot cannot give a process the same one.
        boot: String::new(),
        name: name(pid)?,
    })
}

/// See [`crate::process_start_time`]: the start time `proc_pidinfo` reports, in microseconds; a zombie counts as ended.
pub(crate) fn start_time(pid: u32) -> Option<u64> {
    // SAFETY: an all-zero proc_bsdinfo is a valid buffer for the kernel to fill.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    // SAFETY: the buffer is a live proc_bsdinfo of `size` bytes.
    let n = unsafe {
        libc::proc_pidinfo(
            i32::try_from(pid).ok()?,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    // SZOMB is 5 in <sys/proc.h>.
    if n < size || info.pbi_status == 5 {
        return None;
    }
    Some(info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod sandbox_tests {
    #[test]
    fn the_profile_denies_first_and_allows_listed_folders_after() {
        let sb = crate::Sandbox {
            read: vec!["/usr".into()],
            write: vec![std::env::temp_dir()],
            ..Default::default()
        };
        let p = super::profile(&sb).unwrap();
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
            ..Default::default()
        };
        let p = super::profile(&sb).unwrap();
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
    fn an_offline_process_reaches_this_machine_and_its_own_socket_but_nothing_else() {
        use std::os::unix::net::UnixListener;
        // Short paths: a unix socket path is limited to about 100 bytes.
        let dir = std::path::PathBuf::from(format!("/tmp/agora-off-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("m")).unwrap();
        std::fs::create_dir_all(dir.join("w")).unwrap();
        // Its own mesh socket; another agent's beside it; and one in a folder it may write, as a tmux server's in `/tmp` would be.
        let own = UnixListener::bind(dir.join("m/own")).unwrap();
        let other = UnixListener::bind(dir.join("m/other")).unwrap();
        let tmux = UnixListener::bind(dir.join("w/tmux")).unwrap();
        let v4 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let v6 = std::net::TcpListener::bind("[::1]:0").ok();
        let (p4, p6) = (
            v4.local_addr().unwrap().port(),
            v6.as_ref().map(|l| l.local_addr().unwrap().port()),
        );
        let free = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let sb = crate::Sandbox {
            read: vec!["/usr".into(), "/bin".into()],
            write: vec![dir.join("w"), "/dev".into()],
            offline: true,
            sockets: vec![dir.join("m/own")],
            ..Default::default()
        };
        let p = super::profile(&sb).unwrap();
        // `nc` ships with macOS: -z only connects, -G is the connect timeout, `-U -w 1` connects to a unix socket and gives up after a
        // second of quiet. 192.0.2.1 is TEST-NET (never routed): the sandbox refuses it at once (nc exits silently), while without it
        // the connect waits out the timeout, so the check holds on a machine with no internet too. A refused `nc -l` exits at once.
        let d = dir.display();
        let mut script = format!(
            "s=$(date +%s); nc -z -G 3 192.0.2.1 443 2>/dev/null; e=$(date +%s); \
             [ $((e - s)) -lt 2 ] && echo testnet-refused || echo testnet-other; \
             nc -z -G 2 127.0.0.1 {p4} && echo v4-open || echo v4-closed; \
             nc -U -w 1 {d}/m/own </dev/null 2>/dev/null && echo own-open || echo own-closed; \
             nc -U -w 1 {d}/m/other </dev/null 2>/dev/null && echo other-open || echo other-closed; \
             nc -U -w 1 {d}/w/tmux </dev/null 2>/dev/null && echo tmux-open || echo tmux-closed; \
             nc -l 127.0.0.1 {free} 2>/dev/null & l=$!; sleep 1; \
             kill -0 $l 2>/dev/null && {{ kill $l; echo listen-open; }} || echo listen-refused;"
        );
        if let Some(p6) = p6 {
            script.push_str(&format!(
                " nc -z -G 2 ::1 {p6} && echo v6-open || echo v6-closed;"
            ));
        }
        let out = std::process::Command::new(super::SANDBOX_EXEC)
            .args(["-p", &p, "/bin/sh", "-c", &script])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        for want in [
            "testnet-refused",
            "v4-open",
            "own-open",
            "other-closed",
            "tmux-closed",
            "listen-refused",
        ] {
            assert!(text.contains(want), "{want}: {text}");
        }
        if p6.is_some() {
            assert!(text.contains("v6-open"), "{text}");
        }
        drop((own, other, tmux, v4, v6));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_nodes_own_folders_stay_closed_below_an_allowed_folder_and_its_sockets_too() {
        use std::os::unix::net::UnixListener;
        // #475: a node home under `/tmp`, which every confined agent may write. Short paths: a unix socket path is limited.
        let home = std::path::PathBuf::from(format!("/tmp/agora-deny-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for d in ["data", "m", "wt"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        std::fs::write(home.join("data/trusted-devices"), "web-1 key\n").unwrap();
        let control = UnixListener::bind(home.join("node.sock")).unwrap();
        let own = UnixListener::bind(home.join("m/own")).unwrap();
        let other = UnixListener::bind(home.join("m/other")).unwrap();
        let sb = crate::Sandbox {
            read: vec!["/usr".into(), "/bin".into()],
            write: vec![home.join("wt"), "/tmp".into(), "/dev".into()],
            offline: false,
            sockets: vec![home.join("m/own")],
            deny: vec![home.join("data"), home.join("node.sock"), home.join("m")],
            reopen: vec![],
        };
        let p = super::profile(&sb).unwrap();
        let h = home.display();
        let script = format!(
            "cat {h}/data/trusted-devices >/dev/null 2>&1 && echo read-open || echo read-closed; \
             echo x > {h}/data/new 2>/dev/null && echo write-open || echo write-closed; \
             echo x > {h}/wt/f 2>/dev/null && echo worktree-open || echo worktree-closed; \
             nc -U -w 1 {h}/node.sock </dev/null 2>/dev/null && echo control-open || echo control-closed; \
             nc -U -w 1 {h}/m/other </dev/null 2>/dev/null && echo other-open || echo other-closed; \
             nc -U -w 1 {h}/m/own </dev/null 2>/dev/null && echo own-open || echo own-closed;"
        );
        let out = std::process::Command::new(super::SANDBOX_EXEC)
            .args(["-p", &p, "/bin/sh", "-c", &script])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        for want in [
            "read-closed",
            "write-closed",
            "worktree-open",
            "control-closed",
            "other-closed",
            "own-open",
        ] {
            assert!(text.contains(want), "{want}: {text}\n{p}");
        }
        drop((control, own, other));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_protected_path_made_after_the_profile_is_closed_too() {
        use std::os::unix::net::UnixListener;
        // The node resumes interrupted agents before its control server starts: the control socket (and a data folder not made
        // yet) do not exist when the profile is built, and must be closed once they do.
        let home = std::path::PathBuf::from(format!("/tmp/agora-later-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("wt")).unwrap();
        let sb = crate::Sandbox {
            read: vec!["/usr".into(), "/bin".into()],
            write: vec![home.join("wt"), "/tmp".into(), "/dev".into()],
            offline: false,
            sockets: vec![],
            deny: vec![home.join("node.sock"), home.join("later/data")],
            reopen: vec![],
        };
        let p = super::profile(&sb).unwrap();
        // Named through the resolved parent (`/tmp` is `/private/tmp`) and kept as written below it.
        assert!(
            p.contains(&format!(
                "(subpath \"/private{}/node.sock\")",
                home.display()
            )),
            "{p}"
        );
        assert!(
            p.contains(&format!(
                "(subpath \"/private{}/later/data\")",
                home.display()
            )),
            "{p}"
        );
        let control = UnixListener::bind(home.join("node.sock")).unwrap();
        std::fs::create_dir_all(home.join("later/data")).unwrap();
        std::fs::write(home.join("later/data/trusted-devices"), "web-1 key\n").unwrap();
        let h = home.display();
        let script = format!(
            "cat {h}/later/data/trusted-devices >/dev/null 2>&1 && echo read-open || echo read-closed; \
             nc -U -w 1 {h}/node.sock </dev/null 2>/dev/null && echo control-open || echo control-closed; \
             echo x > {h}/wt/f 2>/dev/null && echo worktree-open || echo worktree-closed;"
        );
        let out = std::process::Command::new(super::SANDBOX_EXEC)
            .args(["-p", &p, "/bin/sh", "-c", &script])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        for want in ["read-closed", "control-closed", "worktree-open"] {
            assert!(text.contains(want), "{want}: {text}\n{p}");
        }
        drop(control);
        let _ = std::fs::remove_dir_all(home);
    }

    /// The review of #477 (a node home under `/tmp`): every way out it named, run by a confined shell. Prints what each did.
    fn run_attacks(home: &std::path::Path, sb: &crate::Sandbox) -> String {
        let p = super::profile(sb).unwrap();
        let h = home.display();
        let w = home.join("worktrees/a").display().to_string();
        let job = format!("agora-escape-{}", std::process::id());
        let script = format!(
            "cd {w}; \
             mv {h} /tmp/agora-moved-$$ 2>/dev/null && echo rename-OPEN && mv /tmp/agora-moved-$$ {h} || echo rename-closed; \
             echo x > {h}/publishers 2>/dev/null && echo publishers-OPEN || echo publishers-closed; \
             echo x > {h}/manifests/evil.toml 2>/dev/null && echo manifests-OPEN || echo manifests-closed; \
             cat {h}/data/trusted-devices >/dev/null 2>&1 && echo read-OPEN || echo read-closed; \
             ls {h}/data >/dev/null 2>&1 && echo list-OPEN || echo list-closed; \
             ln {h}/data/trusted-devices {w}/x 2>/dev/null && cat {w}/x >/dev/null 2>&1 && echo hardlink-OPEN || echo hardlink-closed; \
             cp -c {h}/data/trusted-devices {w}/c 2>/dev/null && echo clone-OPEN || echo clone-closed; \
             launchctl submit -l {job} -- /usr/bin/touch {h}/data/escaped 2>/dev/null && echo launchd-OPEN || echo launchd-closed; \
             ln -s {h}/data {w}/d 2>/dev/null; cat {w}/d/trusted-devices >/dev/null 2>&1 && echo symlink-OPEN || echo symlink-closed; \
             (cd {h} && cat data/trusted-devices >/dev/null 2>&1) && echo relative-OPEN || echo relative-closed; \
             echo x > {w}/ok 2>/dev/null && echo worktree-open || echo worktree-CLOSED; \
             (perl -MIO::Socket::UNIX -e 'IO::Socket::UNIX->new(Type=>SOCK_STREAM(),Local=>$ARGV[0],Listen=>1) or exit 1' {h}/early.sock) 2>/dev/null && echo bind-OPEN || echo bind-closed; \
             nc -U -w 1 {h}/node.sock </dev/null 2>/dev/null && echo connect-OPEN || echo connect-closed; \
             ln {h}/node.sock {w}/s 2>/dev/null && nc -U -w 1 {w}/s </dev/null 2>/dev/null && echo socketlink-OPEN || echo socketlink-closed; \
             cat $(echo {h} | tr a-z A-Z | sed 's#/PRIVATE/TMP#/private/tmp#;s#^/TMP#/tmp#')/data/trusted-devices >/dev/null 2>&1 && echo case-OPEN || echo case-closed; \
             cat /System/Volumes/Data{h}/data/trusted-devices >/dev/null 2>&1 && echo firmlink-OPEN || echo firmlink-closed; \
             cat /System/Volumes/Data/private{h}/data/trusted-devices >/dev/null 2>&1 && echo firmlink2-OPEN || echo firmlink2-closed;"
        );
        let out = std::process::Command::new(super::SANDBOX_EXEC)
            .args(["-p", &p, "/bin/sh", "-c", &script])
            .output()
            .unwrap();
        // Should the submit ever get through, its job is not left behind (launchd would run it again and again).
        let _ = std::process::Command::new("launchctl")
            .args(["remove", &job])
            .status();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn attack_home(tag: &str) -> std::path::PathBuf {
        attack_home_in(std::path::Path::new("/tmp"), tag)
    }

    fn attack_home_in(base: &std::path::Path, tag: &str) -> std::path::PathBuf {
        let home = base.join(format!(".agora-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for d in ["data", "m", "manifests", "worktrees/a", "link"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        std::fs::write(home.join("data/trusted-devices"), "web-1 key\n").unwrap();
        let _ = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(home.join("worktrees/a"))
            .status();
        home
    }

    #[test]
    fn a_node_home_resists_every_way_out_the_review_named_wherever_it_is() {
        // The security reviews of #477: a node home below `/tmp` (writable by every confined agent), below the per-user temp folder
        // (`/var/folders/.../T`) and below the user's home (`~/.agora`), closed as a whole, the agent's own worktree and mesh socket
        // opened again.
        let user_home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        for base in [Some("/tmp".into()), Some(std::env::temp_dir()), user_home]
            .into_iter()
            .flatten()
        {
            ways_out(&base);
        }
    }

    fn ways_out(base: &std::path::Path) {
        let home = attack_home_in(base, "ways");
        let _control = std::os::unix::net::UnixListener::bind(home.join("node.sock")).unwrap();
        let _own = std::os::unix::net::UnixListener::bind(home.join("m/own")).unwrap();
        let sb = crate::Sandbox {
            read: vec![
                "/usr".into(),
                "/bin".into(),
                "/Library".into(),
                "/System".into(),
            ],
            write: vec![home.join("worktrees/a"), "/tmp".into(), "/dev".into()],
            offline: false,
            sockets: vec![home.join("m/own")],
            deny: vec![home.clone()],
            reopen: vec![home.join("worktrees/a")],
        };
        let mut text = run_attacks(&home, &sb);
        let h = home.display();
        let own = std::process::Command::new(super::SANDBOX_EXEC)
            .args([
                "-p",
                &super::profile(&sb).unwrap(),
                "/bin/sh",
                "-c",
                &format!("nc -U -w 1 {h}/m/own </dev/null 2>/dev/null && echo own-open || echo own-CLOSED"),
            ])
            .output()
            .unwrap();
        text.push_str(&String::from_utf8_lossy(&own.stdout));
        assert!(
            !text.contains("OPEN") && !text.contains("CLOSED"),
            "{text}\n{}",
            super::profile(&sb).unwrap()
        );
        for want in [
            // 1 (HIGH): the home cannot be moved away from under its rules.
            "rename-closed",
            // 2 (HIGH): nothing of the home is writable: trust files, manifests.
            "publishers-closed",
            "manifests-closed",
            // 3 (MEDIUM): no socket can be bound there before the node binds its own.
            "bind-closed",
            // And the rest the review asked about: contents, listing, links, a relative path, case, firmlinks, the control socket.
            "read-closed",
            "list-closed",
            "hardlink-closed",
            // Second review (6): a copy-on-write clone (`clonefile`) of a closed file.
            "clone-closed",
            // Second review (5): a job handed to launchd runs outside the profile; launchd refuses one from a confined process.
            "launchd-closed",
            "symlink-closed",
            "relative-closed",
            "connect-closed",
            "socketlink-closed",
            "case-closed",
            "firmlink-closed",
            "firmlink2-closed",
            // What the agent keeps: its worktree and its own mesh socket.
            "worktree-open",
            "own-open",
        ] {
            assert!(text.contains(want), "{want}: {text}");
        }
        assert!(home.exists(), "the home was moved");
        let _ = std::fs::remove_dir_all(home);
    }

    /// What a confined shell can do to the home through `via`, a path the agent made point into it.
    fn through(sb: &crate::Sandbox, home: &std::path::Path, via: &std::path::Path) -> String {
        let (h, v) = (home.display(), via.display());
        let script = format!(
            "cat {h}/data/trusted-devices >/dev/null 2>&1 && echo read-OPEN || echo read-closed; \
             cat {v}/data/trusted-devices >/dev/null 2>&1 && echo via-read-OPEN || echo via-read-closed; \
             ls {v}/data >/dev/null 2>&1 && echo via-list-OPEN || echo via-list-closed; \
             echo x > {v}/publishers 2>/dev/null && echo via-write-OPEN || echo via-write-closed; \
             echo x > {h}/data/policy 2>/dev/null && echo policy-OPEN || echo policy-closed;"
        );
        let out = std::process::Command::new(super::SANDBOX_EXEC)
            .args(["-p", &super::profile(sb).unwrap(), "/bin/sh", "-c", &script])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[test]
    fn a_worktree_swapped_for_a_link_to_the_home_is_not_opened_at_the_next_start() {
        // Second review of #477 (1): the agent removes its worktree and leaves a link to the node's home in its place; the node
        // resumes it with the same worktree path. The home must stay closed.
        let home = attack_home("swap");
        let wt = home.join("worktrees/a");
        std::fs::remove_dir_all(&wt).unwrap();
        std::os::unix::fs::symlink(&home, &wt).unwrap();
        let sb = crate::Sandbox {
            read: vec!["/usr".into(), "/bin".into()],
            write: vec![wt.clone(), "/tmp".into(), "/dev".into()],
            deny: vec![home.clone()],
            reopen: vec![wt.clone()],
            ..Default::default()
        };
        let p = super::profile(&sb).unwrap();
        assert!(
            !p.contains("(allow file-read-data file-read-metadata file-read-xattr file-write*"),
            "{p}"
        );
        let text = through(&sb, &home, &wt);
        assert!(!text.contains("OPEN"), "{text}\n{p}");
        // A link to a folder inside the home (its data) rather than the home itself.
        std::fs::remove_file(&wt).unwrap();
        std::os::unix::fs::symlink(home.join("data"), &wt).unwrap();
        let p = super::profile(&sb).unwrap();
        let (w, h) = (wt.display(), home.display());
        let out = std::process::Command::new(super::SANDBOX_EXEC)
            .args([
                "-p",
                &p,
                "/bin/sh",
                "-c",
                &format!(
                    "cat {w}/trusted-devices >/dev/null 2>&1 && echo data-OPEN || echo data-closed; \
                     cat {h}/data/trusted-devices >/dev/null 2>&1 && echo real-OPEN || echo real-closed; \
                     echo x > {h}/data/policy 2>/dev/null && echo policy-OPEN || echo policy-closed"
                ),
            ])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            !text.contains("OPEN") && text.contains("data-closed"),
            "{text}\n{p}"
        );
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_folder_the_agent_may_write_made_a_link_to_the_home_does_not_open_it() {
        // Second review of #477 (2): an owner's write entry (`~/.claude`) the agent replaced with a link to the node's home, or to a
        // folder inside it.
        let home = attack_home("link");
        let claude = std::path::PathBuf::from(format!("/tmp/agora-claude-{}", std::process::id()));
        let data =
            std::path::PathBuf::from(format!("/tmp/agora-claudedata-{}", std::process::id()));
        let _ = std::fs::remove_file(&claude);
        let _ = std::fs::remove_file(&data);
        std::os::unix::fs::symlink(&home, &claude).unwrap();
        std::os::unix::fs::symlink(home.join("data"), &data).unwrap();
        let sb = crate::Sandbox {
            read: vec!["/usr".into(), "/bin".into(), data.clone()],
            write: vec![home.join("worktrees/a"), claude.clone(), "/dev".into()],
            deny: vec![home.clone()],
            // Even named to be opened again: it resolves to the home itself.
            reopen: vec![home.join("worktrees/a"), claude.clone()],
            ..Default::default()
        };
        let p = super::profile(&sb).unwrap();
        let mut text = through(&sb, &home, &claude);
        text.push_str(&through(&sb, &home, &home));
        let out = std::process::Command::new(super::SANDBOX_EXEC)
            .args([
                "-p",
                &p,
                "/bin/sh",
                "-c",
                &format!(
                    "cat {}/trusted-devices >/dev/null 2>&1 && echo data-OPEN || echo data-closed; \
                     echo x > {}/worktrees/a/f 2>/dev/null && echo worktree-open || echo worktree-CLOSED",
                    data.display(),
                    home.display()
                ),
            ])
            .output()
            .unwrap();
        text.push_str(&String::from_utf8_lossy(&out.stdout));
        assert!(
            !text.contains("OPEN") && !text.contains("CLOSED"),
            "{text}\n{p}"
        );
        assert!(text.contains("worktree-open"), "{text}");
        let _ = std::fs::remove_file(claude);
        let _ = std::fs::remove_file(data);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_protected_path_that_cannot_be_resolved_fails_the_profile() {
        // Second review of #477 (4): a relative node home made the rule silently disappear. Now the agent does not start.
        for deny in ["", "data", "./node.sock"] {
            let sb = crate::Sandbox {
                read: vec!["/usr".into()],
                write: vec![std::env::temp_dir()],
                deny: vec![deny.into()],
                ..Default::default()
            };
            assert!(super::profile(&sb).is_err(), "{deny:?}");
            assert!(
                super::wrap(&sb, "/bin/sh".as_ref(), &[]).is_err(),
                "{deny:?}"
            );
        }
    }

    #[test]
    fn a_profile_that_is_not_offline_says_nothing_about_the_network() {
        let sb = crate::Sandbox::new(vec!["/usr".into()], vec![std::env::temp_dir()]);
        assert!(!super::profile(&sb).unwrap().contains("network"));
    }

    #[test]
    fn a_listed_parent_of_a_private_folder_does_not_open_it() {
        let sb = crate::Sandbox {
            read: vec!["/var".into(), "/usr".into()],
            write: vec![],
            ..Default::default()
        };
        let p = super::profile(&sb).unwrap();
        // `/var` is `/private/var`, which holds `/private/var/folders`; it is left out, `/usr` stays.
        assert!(!p.contains("(subpath \"/private/var\")"), "{p}");
        assert!(
            p.contains("(allow file-read-data (subpath \"/usr\"))"),
            "{p}"
        );
    }
}
