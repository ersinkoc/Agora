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
/// Refuses ([`crate::SandboxRefusal`]) when a `deny` entry is relative or cannot be resolved (a closed folder that silently fell out
/// of the profile would leave the node's home open), when a `reopen` entry is not a plain folder, and when an entry is reached
/// through a link the agent could have made.
///
/// Every path in the profile is where the kernel has it ([`walk`]): Seatbelt matches the path the kernel resolved, so a rule spelled
/// another way (`/System/Volumes/Data/Users/...`, which `realpath` keeps) would match nothing (measured: a deny spelled through
/// that firmlink left the folder readable by both spellings).
fn build_profile(sb: &crate::Sandbox, git_config: Option<&std::path::Path>) -> io::Result<String> {
    let refused = |why| io::Error::new(io::ErrorKind::PermissionDenied, why);
    let mut denied: Vec<std::path::PathBuf> = Vec::new();
    for p in &sb.deny {
        match real_or_future(p).filter(|_| p.is_absolute()) {
            Some(r) => denied.push(r),
            None => return Err(refused(crate::SandboxRefusal::ProtectedPathUnresolved)),
        }
    }
    // #545: an entry is resolved through links when the profile is built, so a link the agent made where it may write would widen
    // its next start (`ln -s ~/.zshrc ~/.claude.json`, `ln -s ~ ~/.claude`, the shared Git folder's `refs` made a link to `~`).
    // The places the agent may write are known first, each where the kernel has it: an entry's own place (its folder resolved, then
    // its name) and where it leads now. Every entry is then walked one component at a time, and a link at one of those places or
    // below one refuses the start ([`agents_link`]): checking a link and resolving the entry are one step, so a link swapped in
    // between cannot slip through. A link the agent cannot have made is followed: one outside every place it may write (an owner's
    // `~/.gitconfig` kept in a dotfiles folder), or the system's (`/tmp` in `/`, which only root may change).
    let mut places: Vec<std::path::PathBuf> = Vec::new();
    for p in sb.write.iter().chain(sb.reopen.iter()) {
        if !p.is_absolute() {
            continue;
        }
        if let (Some(parent), Some(name)) = (p.parent(), p.file_name()) {
            if let Ok(r) = walk(parent, &|_, _| false) {
                places.push(r.path.join(name));
            }
        }
        if let Ok(r) = walk(p, &|_, _| false) {
            places.push(r.path);
        }
    }
    let agents = |at: &std::path::Path, dir: std::os::fd::RawFd| agents_link(at, dir, &places);
    // The agent's worktree must still be the folder the node made: one it replaced with a link (into the node's home, or to any
    // folder it wants written at its next start) refuses the start instead of starting it unable to use its folder, or able to
    // write where the link points.
    let mut reopened: Vec<std::path::PathBuf> = Vec::new();
    for p in &sb.reopen {
        match walk(p, &agents).ok().filter(|r| p.is_absolute() && r.dir) {
            Some(r) => reopened.push(r.path),
            None => return Err(refused(crate::SandboxRefusal::WorktreeNotFolder)),
        }
    }
    // A `read` or `write` entry that resolves below a closed folder (`~/.claude` made a link to the node's home) is left out: the
    // closed folder stays closed whatever the agent left behind for its next start. One that does not exist is left out too.
    let resolve = |list: &[std::path::PathBuf]| -> io::Result<Vec<Walked>> {
        let mut out = Vec::new();
        for p in list.iter().filter(|p| p.is_absolute()) {
            match walk(p, &agents) {
                Ok(r) if !denied.iter().any(|d| r.path.starts_with(d)) => out.push(r),
                Ok(_) | Err(Walk::Missing(_)) | Err(Walk::Failed) => {}
                Err(Walk::AgentsLink) => {
                    return Err(refused(crate::SandboxRefusal::WritableIsLink))
                }
            }
        }
        Ok(out)
    };
    let write_real = resolve(&sb.write)?;
    // #571: a file the agent may write that has another name (a hard link made outside confinement: an agent cannot make one, `ln` is
    // refused) would let it write that other place too, which Seatbelt checks only by the name used. Refused, from the same
    // descriptor the walk resolved. Folders are not scanned at each start (too costly; the accepted risk in T16).
    // The same for the siblings a file entry opens for saving (`<file>.*`, see [`sibling_rule`]): one that is already there with a
    // second name (`state.json.tmp` linked to `~/.zshrc`) would be writable too (review of #592, measured).
    if write_real
        .iter()
        .any(|r| r.file && (r.links > 1 || sibling_has_links(&r.path)))
    {
        return Err(refused(crate::SandboxRefusal::WritableHasLinks));
    }
    let read_real = resolve(&sb.read)?;
    let holds_private = |p: &std::path::Path| {
        PRIVATE
            .iter()
            .map(std::path::Path::new)
            .any(|private| private != p && private.starts_with(p))
    };
    let reads: Vec<String> = read_real
        .iter()
        .chain(write_real.iter())
        .map(|r| &r.path)
        .filter(|p| !holds_private(p))
        .map(|p| quote(p))
        .collect();
    let writes: Vec<String> = write_real.iter().map(|r| quote(&r.path)).collect();
    let siblings: Vec<String> = write_real
        .iter()
        .filter(|r| r.file)
        .filter_map(|r| sibling_rule(&r.path))
        .collect();
    let private: Vec<String> = PRIVATE
        .iter()
        .map(|p| quote(std::path::Path::new(p)))
        .collect();
    // Preferences (`defaults`, CFPreferences) are read and written by `cfprefsd`, not by the process: it checks these operations and,
    // only when they are denied, the file rules below, so without the deny any domain under `~/Library/Preferences` is open (F201).
    // The global domain stays readable (language, locale), as the system does.
    let mut out = format!(
        "(version 1)\n(allow default)\n(deny file-write*)\n(deny file-read-data {})\n\
         (deny user-preference-read user-preference-write)\n\
         (allow user-preference-read (preference-domain \"kCFPreferencesAnyApplication\"))\n",
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
    // ADR-0083: unix sockets only where the agent was given them. `/tmp` holds an SSH agent's, a tmux server's and the session bus's
    // sockets; each is a way out of the folder rules. Every unix socket is refused, then the allow-list reopened (the agent's own mesh
    // socket, its worktree and private temp folder, the system's name resolver, and what the owner listed in `sandbox.sockets`).
    // `subpath` of a socket file is that file, and of a folder everything in it. The node's own folders are closed again below.
    // The form `(subpath "/")` names every absolute path, so it needs no filter-less spelling of `remote unix-socket`.
    // `remote unix-socket` takes exactly one filter, and `quote` already returns one (`(subpath "...")`): nesting a second `subpath` is
    // a profile `sandbox-exec` refuses to parse (exit 65), which the agent sees as never starting (measured on a Mac, 2026-11-17).
    if sb.confine_sockets && !sb.offline {
        out.push_str("(deny network-outbound (remote unix-socket (subpath \"/\")))\n");
        let mut allowed: Vec<String> = SYSTEM_SOCKETS
            .iter()
            .map(|p| quote(std::path::Path::new(p)))
            .collect();
        allowed.extend(
            sb.sockets
                .iter()
                .filter_map(|p| real_or_new(p))
                .map(|p| quote(&p)),
        );
        let to: Vec<String> = allowed
            .iter()
            .map(|q| format!("(remote unix-socket {q})"))
            .collect();
        out.push_str(&format!("(allow network-outbound {})\n", to.join(" ")));
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
        let writable: Vec<&std::path::PathBuf> = write_real.iter().map(|r| &r.path).collect();
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
        let inside: Vec<&std::path::PathBuf> = reopened
            .iter()
            .filter(|w| denied.iter().any(|d| w.starts_with(d) && *w != d))
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
    // ADR-0098: reopen only the generated file after the deny on its parent. The
    // parent metadata grants allow traversal and do not grant directory listing.
    if let Some(file) = git_config {
        out.push_str(&format!(
            "(allow file-read-data file-read-metadata {})\n",
            literal(file)
        ));
        let parents: Vec<_> = file
            .ancestors()
            .skip(1)
            .filter(|p| denied.iter().any(|d| p.starts_with(d)))
            .map(literal)
            .collect();
        if !parents.is_empty() {
            out.push_str(&format!(
                "(allow file-read-metadata {})\n",
                parents.join(" ")
            ));
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

pub(crate) fn git_config_path(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let found = walk(path, &|_, _| true).ok()?;
    (found.file && found.links == 1).then_some(found.path)
}

/// An entry where the kernel has it (see [`walk`]).
struct Walked {
    path: std::path::PathBuf,
    dir: bool,
    file: bool,
    /// How many names the file has (`st_nlink`), read from the descriptor the walk ended on.
    links: u64,
}

/// Why [`walk`] stopped.
enum Walk {
    /// A component does not exist: the path the kernel has for the folders that do, and the rest as it was written.
    Missing(std::path::PathBuf),
    /// A link the agent could have made ([`agents_link`]).
    AgentsLink,
    /// Not walked to the end (no access, too many links, not a folder on the way).
    Failed,
}

/// Where the absolute path `p` really is, walked from `/` one component at a time: each opened with `O_NOFOLLOW` relative to the
/// folder before it (`O_EVTONLY`: no read access needed), each link read there (`readlinkat`) and followed by hand after `agents`
/// was asked about it (its place, and the folder holding it), and the end's path taken from its descriptor (`F_GETPATH`). Nothing is
/// looked up twice by name, so what is checked is what is resolved. At most 40 links, as the kernel.
fn walk(
    p: &std::path::Path,
    agents: &dyn Fn(&std::path::Path, std::os::fd::RawFd) -> bool,
) -> Result<Walked, Walk> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::path::Component;
    const FLAGS: libc::c_int =
        libc::O_EVTONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
    let open = |dir: Option<&OwnedFd>, name: &std::ffi::OsStr| -> io::Result<OwnedFd> {
        let c = std::ffi::CString::new(name.as_bytes())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let at = dir.map_or(libc::AT_FDCWD, |d| d.as_raw_fd());
        // SAFETY: `c` is NUL-terminated and lives for the call; the descriptor returned is owned here.
        let fd = unsafe { libc::openat(at, c.as_ptr(), FLAGS) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `fd` was just opened and is owned by nothing else.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    };
    let path_of = |fd: &OwnedFd| -> io::Result<std::path::PathBuf> {
        let mut buf = vec![0u8; libc::PATH_MAX as usize];
        // SAFETY: F_GETPATH writes a NUL-terminated path of at most PATH_MAX bytes into `buf`, which holds that many.
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETPATH, buf.as_mut_ptr()) } == -1 {
            return Err(io::Error::last_os_error());
        }
        let len = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
        Ok(std::path::PathBuf::from(std::ffi::OsStr::from_bytes(
            &buf[..len],
        )))
    };
    let failed = |_: io::Error| Walk::Failed;
    if !p.is_absolute() {
        return Err(failed(io::Error::from(io::ErrorKind::InvalidInput)));
    }
    let root = std::ffi::OsStr::new("/");
    let mut at = open(None, root).map_err(failed)?;
    // The components still to walk, the next one last.
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    let push = |rest: &mut Vec<std::ffi::OsString>, p: &std::path::Path| {
        for c in p.components().rev() {
            match c {
                Component::Normal(n) => rest.push(n.to_os_string()),
                Component::ParentDir => rest.push("..".into()),
                Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
            }
        }
    };
    push(&mut rest, p);
    let mut links = 0;
    while let Some(name) = rest.pop() {
        match open(Some(&at), &name) {
            Ok(next) => at = next,
            Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
                let place = path_of(&at).map_err(failed)?.join(&name);
                if agents(&place, at.as_raw_fd()) {
                    return Err(Walk::AgentsLink);
                }
                links += 1;
                if links > 40 {
                    return Err(failed(e));
                }
                let c = std::ffi::CString::new(name.as_bytes())
                    .map_err(|_| failed(io::Error::from(io::ErrorKind::InvalidInput)))?;
                let mut buf = vec![0u8; libc::PATH_MAX as usize];
                // SAFETY: `c` is NUL-terminated; readlinkat writes at most `buf.len()` bytes into `buf`.
                let n = unsafe {
                    libc::readlinkat(
                        at.as_raw_fd(),
                        c.as_ptr(),
                        buf.as_mut_ptr().cast(),
                        buf.len(),
                    )
                };
                if n < 0 {
                    return Err(failed(io::Error::last_os_error()));
                }
                buf.truncate(n as usize);
                let target = std::path::PathBuf::from(std::ffi::OsString::from_vec(buf));
                if target.is_absolute() {
                    at = open(None, root).map_err(failed)?;
                }
                push(&mut rest, &target);
            }
            Err(e) if e.raw_os_error() == Some(libc::ENOENT) => {
                let mut missing = path_of(&at).map_err(failed)?.join(&name);
                while let Some(n) = rest.pop() {
                    missing.push(n);
                }
                return Err(Walk::Missing(missing));
            }
            // The last component is something `open` refuses (a unix socket): its place is its folder's, joined with its name, as long
            // as it is no link (checked through the same folder).
            Err(e) if rest.is_empty() => {
                let c = std::ffi::CString::new(name.as_bytes()).map_err(|_| Walk::Failed)?;
                // SAFETY: an all-zero `stat` is a valid value to be overwritten.
                let mut st: libc::stat = unsafe { std::mem::zeroed() };
                // SAFETY: `c` is NUL-terminated; `st` is a valid `stat` to write.
                let r = unsafe {
                    libc::fstatat(
                        at.as_raw_fd(),
                        c.as_ptr(),
                        &mut st,
                        libc::AT_SYMLINK_NOFOLLOW,
                    )
                };
                if r != 0 || st.st_mode & libc::S_IFMT == libc::S_IFLNK {
                    return Err(failed(e));
                }
                return Ok(Walked {
                    path: path_of(&at).map_err(failed)?.join(&name),
                    dir: false,
                    file: st.st_mode & libc::S_IFMT == libc::S_IFREG,
                    links: u64::from(st.st_nlink),
                });
            }
            Err(e) => return Err(failed(e)),
        }
    }
    let path = path_of(&at).map_err(failed)?;
    let meta = std::fs::File::from(at).metadata().map_err(failed)?;
    Ok(Walked {
        path,
        dir: meta.is_dir(),
        file: meta.is_file(),
        links: std::os::unix::fs::MetadataExt::nlink(&meta),
    })
}

/// Whether a link found at `place` (in the folder open as `dir`) is one the agent could have made: below a place it may write (it
/// could also have changed that folder's mode), or at one of those places itself while this user may write the folder holding it
/// (`~/.claude` in `~`; not `/tmp` in `/`, which only root may change). The places are where the kernel has them, so another
/// spelling of the same folder (`/private/tmp`, `/System/Volumes/Data/Users/...`, other upper or lower case) is the same place.
fn agents_link(
    place: &std::path::Path,
    dir: std::os::fd::RawFd,
    places: &[std::path::PathBuf],
) -> bool {
    if places.iter().any(|w| place != w && place.starts_with(w)) {
        return true;
    }
    places.iter().any(|w| place == w)
        // SAFETY: `dir` is an open folder for the call; "." is a NUL-terminated literal.
        && unsafe { libc::faccessat(dir, c".".as_ptr(), libc::W_OK, libc::AT_EACCESS) } == 0
}

/// Whether a file next to `file` whose name starts with its name and a dot (what [`sibling_rule`] lets the agent write) is a
/// regular file with more than one name. A folder that cannot be read counts as holding one: the start is refused rather than
/// opened unchecked.
fn sibling_has_links(file: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else {
        return true;
    };
    let mut prefix = name.as_bytes().to_vec();
    prefix.push(b'.');
    let Ok(entries) = std::fs::read_dir(dir) else {
        return true;
    };
    entries.filter_map(Result::ok).any(|e| {
        e.file_name().as_bytes().starts_with(&prefix)
            && std::fs::symlink_metadata(e.path()).is_ok_and(|m| m.is_file() && m.nlink() > 1)
    })
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

/// `p` where the kernel has it as far as it exists (links in its existing part followed: `/tmp` is `/private/tmp`), the rest of it as
/// written. `None` when it cannot be walked, or when the missing rest steps up (`..`), which no rule could name.
fn real_or_future(p: &std::path::Path) -> Option<std::path::PathBuf> {
    match walk(p, &|_, _| false) {
        Ok(r) => Some(r.path),
        Err(Walk::Missing(m)) if !m.components().any(|c| c == std::path::Component::ParentDir) => {
            Some(m)
        }
        Err(_) => None,
    }
}

/// The system's unix sockets every confined agent may use under [`crate::Sandbox::confine_sockets`]: the name resolver (every network
/// lookup goes through it; `/var/run` is `/private/var/run`, and a profile names the path the kernel resolved). A socket a real tool
/// needs that is not here is a broken tool the owner lists in `sandbox.sockets` (ADR-0083).
const SYSTEM_SOCKETS: &[&str] = &["/private/var/run/mDNSResponder"];

/// `p` where the kernel has it, or, when it does not exist yet (a socket made later), its folder where the kernel has it and its name.
fn real_or_new(p: &std::path::Path) -> Option<std::path::PathBuf> {
    match walk(p, &|_, _| false) {
        Ok(r) => Some(r.path),
        Err(_) => Some(
            walk(p.parent()?, &|_, _| false)
                .ok()?
                .path
                .join(p.file_name()?),
        ),
    }
}

/// [`crate::rename_no_replace`] on macOS: `renamex_np(RENAME_EXCL)`. `ENOTSUP` and `EINVAL` (a file system without it) are
/// `Unsupported`.
pub(crate) fn rename_no_replace(from: &std::path::Path, to: &std::path::Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = |p: &std::path::Path| {
        std::ffi::CString::new(p.as_os_str().as_bytes())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
    };
    let (from, to) = (c(from)?, c(to)?);
    // SAFETY: two NUL-terminated paths that outlive the call.
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } == 0 {
        return Ok(());
    }
    let e = io::Error::last_os_error();
    match e.raw_os_error() {
        Some(libc::ENOTSUP) | Some(libc::EINVAL) => {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
        _ => Err(e),
    }
}

#[cfg(test)]
fn profile(sb: &crate::Sandbox) -> io::Result<String> {
    build_profile(sb, sb.git_config_file().as_deref())
}

/// `program args...` rewritten to run under `sandbox-exec` with the profile for `sb`.
pub(crate) fn wrap(
    sb: &crate::Sandbox,
    program: &std::ffi::OsStr,
    args: &[std::ffi::OsString],
) -> io::Result<(
    std::ffi::OsString,
    Vec<std::ffi::OsString>,
    Option<std::path::PathBuf>,
)> {
    let git_config = sb.git_config_file();
    let mut v: Vec<std::ffi::OsString> = vec![
        "-p".into(),
        build_profile(sb, git_config.as_deref())?.into(),
        "--".into(),
        program.into(),
    ];
    v.extend(args.iter().cloned());
    Ok((SANDBOX_EXEC.into(), v, git_config))
}

/// See [`crate::desktop_notify`]: not yet on macOS. The system's own notifier for a script (`osascript`) posts as *Script Editor*:
/// it shows only when Script Editor may notify, and a click opens Script Editor instead of Agora (measured on macOS 27.2), so nothing
/// is posted. The app's own notification (`UNUserNotificationCenter`) is the planned way (#296 follow-up).
pub(crate) fn desktop_notify(title: &str, body: &str) -> io::Result<()> {
    let _ = (title, body);
    Err(io::Error::from(io::ErrorKind::Unsupported))
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

/// See [`crate::end_with_supervisor`]: macOS has no parent-death signal, so a thread waits on a `kqueue` for the supervisor's exit
/// (`EVFILT_PROC`/`NOTE_EXIT`) and then sends `SIGTERM` to this process. When the supervisor ended before the watch was set (it is
/// gone, or this process has another parent already), the signal is sent at once.
pub(crate) fn end_with(supervisor: u32) {
    let Ok(pid) = libc::pid_t::try_from(supervisor) else {
        return;
    };
    // SAFETY: plain system call.
    let kq = unsafe { libc::kqueue() };
    if kq < 0 {
        return;
    }
    // SAFETY: an all-zero kevent is valid; the fields below make it a one-shot exit watch on `pid`.
    let mut watch: libc::kevent = unsafe { std::mem::zeroed() };
    watch.ident = pid as libc::uintptr_t;
    watch.filter = libc::EVFILT_PROC;
    watch.flags = libc::EV_ADD | libc::EV_ONESHOT;
    watch.fflags = libc::NOTE_EXIT;
    // SAFETY: one valid change, no output buffer.
    let added = unsafe { libc::kevent(kq, &watch, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
    // SAFETY: plain system calls.
    if added < 0 || unsafe { libc::getppid() } != pid {
        unsafe {
            libc::close(kq);
            libc::raise(libc::SIGTERM);
        }
        return;
    }
    let _ = std::thread::Builder::new()
        .name("agora-supervisor-watch".into())
        .spawn(move || loop {
            // SAFETY: an all-zero kevent is a valid output buffer of one entry.
            let mut fired: libc::kevent = unsafe { std::mem::zeroed() };
            // SAFETY: no changes, one output entry, no timeout.
            let n =
                unsafe { libc::kevent(kq, std::ptr::null(), 0, &mut fired, 1, std::ptr::null()) };
            if n > 0 {
                // SAFETY: plain system call.
                unsafe { libc::raise(libc::SIGTERM) };
                return;
            }
            if n < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                return;
            }
        });
}

// ----- ADR-0136: the machine's CPU, memory and disks -----

/// The host port, asked for once: each `mach_host_self` call adds a user reference to its send right, and a reader that runs every 30 s
/// for weeks would pile them up.
#[allow(deprecated)] // `libc` prefers the `mach2` crate; a few stable calls are not worth a new dependency
fn host_port() -> libc::mach_port_t {
    static HOST: std::sync::OnceLock<libc::mach_port_t> = std::sync::OnceLock::new();
    // SAFETY: no arguments; it returns a send right to this host's port, kept for the life of the process.
    *HOST.get_or_init(|| unsafe { libc::mach_host_self() })
}

/// A numeric `sysctl` value. A 4-byte one fills the low half of the zeroed `u64` (both macOS targets are little-endian).
fn sysctl_u64(name: &CStr) -> io::Result<u64> {
    let mut value: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    // SAFETY: a NUL-terminated name, and an out-buffer of `len` bytes that outlives the call; nothing is written.
    let r = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut value as *mut u64).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    if len != 4 && len != 8 {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    Ok(value)
}

/// The machine's CPU ticks (user, system, idle, nice; summed over every CPU) as the kernel counts them, in 32 bits.
#[allow(deprecated)] // see `host_port`
fn cpu_ticks() -> io::Result<[u32; 4]> {
    // SAFETY: an all-zero host_cpu_load_info is a valid out-buffer.
    let mut info: libc::host_cpu_load_info = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
    // SAFETY: the buffer is a live host_cpu_load_info, and `count` says how many integers it holds.
    let r = unsafe {
        libc::host_statistics(
            host_port(),
            libc::HOST_CPU_LOAD_INFO,
            (&mut info as *mut libc::host_cpu_load_info).cast(),
            &mut count,
        )
    };
    if r != libc::KERN_SUCCESS {
        return Err(io::Error::from(io::ErrorKind::Other));
    }
    Ok(info.cpu_ticks)
}

/// `(free, inactive, purgeable)` pages from the kernel's VM statistics.
#[allow(deprecated)] // see `host_port`
fn vm_pages() -> io::Result<(u32, u32, u32)> {
    // SAFETY: an all-zero vm_statistics64 is a valid out-buffer.
    let mut info: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    // SAFETY: the buffer is a live vm_statistics64, and `count` says how many integers it holds.
    let r = unsafe {
        libc::host_statistics64(
            host_port(),
            libc::HOST_VM_INFO64,
            (&mut info as *mut libc::vm_statistics64).cast(),
            &mut count,
        )
    };
    if r != libc::KERN_SUCCESS {
        return Err(io::Error::from(io::ErrorKind::Other));
    }
    Ok((info.free_count, info.inactive_count, info.purgeable_count))
}

/// The kernel's tick counters are 32 bits and wrap (summed over 10 CPUs at 100 Hz, after about 50 days), while [`crate::MachineStats`]
/// promises counters since boot that two readings subtract. `last` holds the previous raw reading and the running sums; each new
/// reading adds the wrapping difference from the last one, so the sums never go back as long as readings are less than a wrap apart.
fn carry(last: &mut Option<([u32; 4], [u64; 4])>, raw: [u32; 4]) -> [u64; 4] {
    let (prev, mut sums) = last.unwrap_or(([0; 4], [0; 4]));
    for ((sum, now), before) in sums.iter_mut().zip(raw).zip(prev) {
        *sum = sum.saturating_add(u64::from(now.wrapping_sub(before)));
    }
    *last = Some((raw, sums));
    sums
}

/// `(busy, total)` ticks from the four counters in the kernel's order (user, system, idle, nice).
fn busy_and_total(t: [u64; 4]) -> (u64, u64) {
    let [user, system, idle, nice] = t;
    let busy = user.saturating_add(system).saturating_add(nice);
    (busy, busy.saturating_add(idle))
}

/// What the system can give without swapping: free (which counts speculative pages), inactive and purgeable pages, never more
/// than the memory there is.
fn available_bytes(free: u32, inactive: u32, purgeable: u32, page: u64, total: u64) -> u64 {
    (u64::from(free) + u64::from(inactive) + u64::from(purgeable))
        .saturating_mul(page)
        .min(total)
}

/// See [`crate::machine_stats`]: `host_statistics(HOST_CPU_LOAD_INFO)` ticks, `hw.logicalcpu`, `hw.memsize` and
/// `host_statistics64(HOST_VM_INFO64)` pages.
pub(crate) fn machine_stats() -> io::Result<crate::MachineStats> {
    static LAST: std::sync::Mutex<Option<([u32; 4], [u64; 4])>> = std::sync::Mutex::new(None);
    let raw = cpu_ticks()?;
    let ticks = {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        carry(&mut last, raw)
    };
    let (cpu_busy_ticks, cpu_total_ticks) = busy_and_total(ticks);
    let cpu_count = u32::try_from(sysctl_u64(c"hw.logicalcpu")?)
        .unwrap_or(u32::MAX)
        .max(1);
    let memory_total_bytes = sysctl_u64(c"hw.memsize")?;
    // SAFETY: plain call.
    let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) })
        .ok()
        .filter(|p| *p > 0)
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
    let (free, inactive, purgeable) = vm_pages()?;
    Ok(crate::MachineStats {
        cpu_count,
        cpu_busy_ticks,
        cpu_total_ticks,
        memory_total_bytes,
        memory_available_bytes: available_bytes(
            free,
            inactive,
            purgeable,
            page,
            memory_total_bytes,
        ),
    })
}

/// One mounted file system as `getfsstat` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Mount {
    mount: String,
    fstype: String,
    flags: u32,
    fsid: [i32; 2],
}

/// The file system id of a `statfs` (the libc type keeps its two numbers private).
fn fsid_of(st: &libc::statfs) -> [i32; 2] {
    // SAFETY: `fsid_t` is a `repr(C)` struct of exactly two `i32`s.
    unsafe { std::mem::transmute::<libc::fsid_t, [i32; 2]>(st.f_fsid) }
}

fn c_text(chars: &[libc::c_char]) -> String {
    let bytes: Vec<u8> = chars
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Every mounted file system, from `getfsstat` into a buffer of our own (`getmntinfo` hands back a static one).
fn mounts() -> io::Result<Vec<Mount>> {
    // SAFETY: a null buffer asks for the count only.
    let n = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
    let n = usize::try_from(n).map_err(|_| io::Error::last_os_error())?;
    // Room for a few mounts that appear between the two calls.
    let room = n + 4;
    // SAFETY: an all-zero statfs is a valid out-buffer entry.
    let mut buf: Vec<libc::statfs> = vec![unsafe { std::mem::zeroed() }; room];
    let bytes = libc::c_int::try_from(room * std::mem::size_of::<libc::statfs>())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    // SAFETY: `buf` holds `bytes` bytes of statfs entries and outlives the call.
    let got = unsafe { libc::getfsstat(buf.as_mut_ptr(), bytes, libc::MNT_NOWAIT) };
    let got = usize::try_from(got).map_err(|_| io::Error::last_os_error())?;
    buf.truncate(got.min(room));
    Ok(buf
        .iter()
        .map(|st| Mount {
            mount: c_text(&st.f_mntonname),
            fstype: c_text(&st.f_fstypename),
            flags: st.f_flags,
            fsid: fsid_of(st),
        })
        .collect())
}

/// The mount where the user's files are: hidden from the Finder like the system's own volumes, but it is the one that fills.
const DATA_VOLUME: &str = "/System/Volumes/Data";

/// Whether a mount holds data that can run out: local; not a pseudo or automount file system; not read-only (a disk image or the
/// recovery volume never fills, and a full one would only raise a false alarm); not hidden from the Finder (`nobrowse`: the
/// system's VM, Preboot, Update... volumes); and not a snapshot. `/` (the sealed system snapshot, read-only) and [`DATA_VOLUME`]
/// are kept: they share the disk the user fills.
fn holds_data(m: &Mount) -> bool {
    let flag = |f: libc::c_int| m.flags & (f as u32) != 0;
    if m.mount == "/" || m.mount == DATA_VOLUME {
        return flag(libc::MNT_LOCAL);
    }
    flag(libc::MNT_LOCAL)
        && !matches!(m.fstype.as_str(), "devfs" | "autofs" | "nullfs")
        && !flag(libc::MNT_RDONLY)
        && !flag(libc::MNT_DONTBROWSE)
        && !flag(libc::MNT_SNAPSHOT)
}

/// The data mounts, one per file system id, each with the indexes of the marks whose file system id it has, ordered as
/// [`crate::disks`] says: `/` first, then those holding a mark, then by mount point; at most 8.
fn pick_mounts(all: &[Mount], mark_ids: &[Option<[i32; 2]>]) -> Vec<(String, Vec<usize>)> {
    let mut picked: Vec<(&Mount, Vec<usize>)> = Vec::new();
    for m in all.iter().filter(|m| holds_data(m)) {
        if !picked.iter().any(|(p, _)| p.fsid == m.fsid) {
            picked.push((m, Vec::new()));
        }
    }
    for (k, id) in mark_ids.iter().enumerate() {
        if let Some((_, holds)) = id.and_then(|id| picked.iter_mut().find(|(p, _)| p.fsid == id)) {
            holds.push(k);
        }
    }
    picked.sort_by(|a, b| {
        let rank = |g: &(&Mount, Vec<usize>)| (g.0.mount != "/", g.1.is_empty());
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.0.mount.cmp(&b.0.mount))
    });
    picked
        .into_iter()
        .take(8)
        .map(|(m, holds)| (m.mount.clone(), holds))
        .collect()
}

/// `statfs` of `path`.
fn stat_fs(path: &std::path::Path) -> io::Result<libc::statfs> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: an all-zero statfs is a valid out-buffer.
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: a NUL-terminated path and a struct that outlive the call.
    crate::unix::cvt(unsafe { libc::statfs(c.as_ptr(), &mut st) })?;
    Ok(st)
}

/// See [`crate::disks`]: `getfsstat`, the mounts [`holds_data`] keeps, one per file system id. A mark lies on the file system whose id
/// its own `statfs` gives, so a folder under `/Users` counts on [`DATA_VOLUME`], where it really is (a firmlink makes it look under
/// `/`). The figures are a fresh reading of each mount, as on Linux.
pub(crate) fn disks(marks: &[&std::path::Path]) -> io::Result<Vec<crate::DiskSpace>> {
    let all = mounts()?;
    let mark_ids: Vec<Option<[i32; 2]>> = marks
        .iter()
        .map(|m| stat_fs(m).ok().map(|st| fsid_of(&st)))
        .collect();
    Ok(pick_mounts(&all, &mark_ids)
        .into_iter()
        .filter_map(|(mount, holds)| {
            let (total_bytes, free_bytes) =
                crate::unix::disk_space(std::path::Path::new(&mount)).ok()?;
            (total_bytes > 0).then_some(crate::DiskSpace {
                mount,
                total_bytes,
                free_bytes,
                holds,
            })
        })
        .collect())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod machine_tests {
    use super::*;

    #[test]
    fn cpu_counters_go_on_past_a_32_bit_wrap_and_memory_counts_what_can_be_given() {
        let mut last = None;
        assert_eq!(carry(&mut last, [100, 50, 800, 0]), [100, 50, 800, 0]);
        // The idle counter passes the top of 32 bits and reads 20 again: it goes on from 2^32.
        let near = u32::MAX - 9;
        assert_eq!(carry(&mut last, [110, 60, near, 0])[2], u64::from(near));
        let after = carry(&mut last, [120, 70, 20, 5]);
        assert_eq!(after, [120, 70, (1u64 << 32) + 20, 5]);
        assert_eq!(busy_and_total([100, 50, 800, 10]), (160, 960));
        let page = 16_384;
        assert_eq!(available_bytes(10, 20, 5, page, 1 << 40), 35 * page);
        assert_eq!(
            available_bytes(u32::MAX, u32::MAX, 0, page, 1 << 30),
            1 << 30
        );
    }

    fn mount(at: &str, fstype: &str, flags: libc::c_int, id: i32) -> Mount {
        Mount {
            mount: at.to_string(),
            fstype: fstype.to_string(),
            flags: flags as u32,
            fsid: [id, 26],
        }
    }

    #[test]
    fn only_the_disks_that_fill_count_once_each_with_their_marks() {
        use libc::{MNT_DONTBROWSE as HIDDEN, MNT_LOCAL as LOCAL, MNT_RDONLY as RO};
        // As `mount` lists them on an Apple silicon Mac with a disk image and an external drive.
        let all = [
            mount("/", "apfs", LOCAL | RO | libc::MNT_ROOTFS, 1),
            mount("/dev", "devfs", LOCAL | HIDDEN, 2),
            mount("/System/Volumes/VM", "apfs", LOCAL | HIDDEN, 3),
            mount("/System/Volumes/Preboot", "apfs", LOCAL | HIDDEN, 4),
            mount("/System/Volumes/Update", "apfs", LOCAL | HIDDEN, 5),
            mount("/System/Volumes/xarts", "apfs", LOCAL | HIDDEN, 6),
            mount("/System/Volumes/iSCPreboot", "apfs", LOCAL | HIDDEN, 7),
            mount("/System/Volumes/Hardware", "apfs", LOCAL | HIDDEN, 8),
            mount(DATA_VOLUME, "apfs", LOCAL | HIDDEN, 9),
            mount("/System/Volumes/Data/home", "autofs", HIDDEN, 10),
            mount("/Volumes/Recovery", "apfs", LOCAL | HIDDEN, 11),
            mount("/Volumes/An App", "hfs", LOCAL | RO, 12),
            mount("/Volumes/Backup", "apfs", LOCAL, 13),
            mount("/Volumes/Server", "smbfs", 0, 14),
            // The same file system seen twice counts once.
            mount("/Volumes/Backup again", "apfs", LOCAL, 13),
        ];
        // A mark on the data volume, one on nothing listed, one on the external drive.
        let marks = [Some([9, 26]), None, Some([13, 26])];
        assert_eq!(
            pick_mounts(&all, &marks),
            [
                ("/".to_string(), vec![]),
                ("/System/Volumes/Data".to_string(), vec![0]),
                ("/Volumes/Backup".to_string(), vec![2]),
            ]
        );
        // With no marks: the root first, then by mount point.
        assert_eq!(
            pick_mounts(&all, &[])
                .into_iter()
                .map(|(m, _)| m)
                .collect::<Vec<_>>(),
            ["/", "/System/Volumes/Data", "/Volumes/Backup"]
        );
    }

    #[test]
    fn this_machine_reports_its_figures() {
        let a = machine_stats().unwrap();
        assert!(
            a.cpu_count >= 1
                && a.cpu_total_ticks >= a.cpu_busy_ticks
                && a.memory_total_bytes > 0
                && a.memory_available_bytes <= a.memory_total_bytes,
            "{a:?}"
        );
        let b = machine_stats().unwrap();
        assert!(b.cpu_total_ticks >= a.cpu_total_ticks, "{a:?} {b:?}");
        let here = std::env::temp_dir();
        let d = disks(&[here.as_path()]).unwrap();
        assert!(
            !d.is_empty()
                && d[0].mount == "/"
                && d.iter().all(|x| x.total_bytes >= x.free_bytes)
                && d.iter().any(|x| x.holds == [0]),
            "{d:?}"
        );
    }
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
    fn a_worktree_swapped_for_a_link_refuses_the_next_start() {
        // Second review of #477 (1), and the third's ask to refuse rather than start: the agent removes its worktree and leaves a
        // link in its place (to the node's home, into it, or anywhere else it would like written); the node starts it again with the
        // same worktree path.
        let home = attack_home("swap");
        let wt = home.join("worktrees/a");
        let elsewhere =
            std::path::PathBuf::from(format!("/tmp/agora-elsewhere-{}", std::process::id()));
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::remove_dir_all(&wt).unwrap();
        let sb = crate::Sandbox {
            read: vec!["/usr".into(), "/bin".into()],
            write: vec![wt.clone(), "/tmp".into(), "/dev".into()],
            deny: vec![home.clone()],
            reopen: vec![wt.clone()],
            ..Default::default()
        };
        for target in [home.clone(), home.join("data"), elsewhere.clone()] {
            std::os::unix::fs::symlink(&target, &wt).unwrap();
            let e = super::profile(&sb).unwrap_err();
            assert_eq!(
                crate::sandbox_refusal(&e),
                Some(crate::SandboxRefusal::WorktreeNotFolder),
                "{target:?}"
            );
            assert!(super::wrap(&sb, "/bin/sh".as_ref(), &[]).is_err());
            std::fs::remove_file(&wt).unwrap();
        }
        // Gone, or a file: refused too.
        assert!(super::profile(&sb).is_err());
        std::fs::write(&wt, "x").unwrap();
        assert!(super::profile(&sb).is_err());
        std::fs::remove_file(&wt).unwrap();
        // And were it a link only in the write list, the start is refused too (#545).
        std::os::unix::fs::symlink(home.join("data"), &wt).unwrap();
        let only_written = crate::Sandbox {
            reopen: vec![],
            ..sb.clone()
        };
        assert_eq!(
            crate::sandbox_refusal(&super::profile(&only_written).unwrap_err()),
            Some(crate::SandboxRefusal::WritableIsLink)
        );
        let _ = std::fs::remove_dir_all(home);
        let _ = std::fs::remove_dir_all(elsewhere);
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
            reopen: vec![home.join("worktrees/a")],
            ..Default::default()
        };
        // The write entry made a link: refused (#545).
        assert_eq!(
            crate::sandbox_refusal(&super::profile(&sb).unwrap_err()),
            Some(crate::SandboxRefusal::WritableIsLink)
        );
        // A read entry that is a link outside every write entry is followed, and opens nothing of the home all the same.
        let sb = crate::Sandbox {
            write: vec![home.join("worktrees/a"), "/dev".into()],
            ..sb
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
    fn a_link_the_agent_makes_where_it_may_write_refuses_its_next_start() {
        // #545: owner write entries were followed through links. The agent replaces one with a link (a file to `.zshrc`, a folder to
        // the user's home, a folder on the way to a read entry, the shared Git folder's `refs`), and was given where it points at its
        // next start. A scratch "user home" under /tmp stands in for `~`.
        let u = std::path::PathBuf::from(format!("/tmp/agora-545-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&u);
        for d in [".claude/a", "repo/.git/refs", "secret"] {
            std::fs::create_dir_all(u.join(d)).unwrap();
        }
        std::fs::write(u.join(".claude.json"), "{}").unwrap();
        std::fs::write(u.join(".zshrc"), "# rc\n").unwrap();
        std::fs::write(u.join(".claude/a/b.json"), "{}").unwrap();
        let sb = crate::Sandbox {
            read: vec!["/usr".into(), "/bin".into(), u.join(".claude/a/b.json")],
            write: vec![
                u.join(".claude.json"),
                u.join(".claude"),
                u.join("repo/.git/refs"),
                "/tmp".into(),
                "/dev".into(),
            ],
            ..Default::default()
        };
        // Before: allowed, and `/tmp` (itself a link, in `/`) is fine.
        let p = super::profile(&sb).unwrap();
        let (h, run) = (u.display(), |p: &str, script: &str| {
            let out = std::process::Command::new(super::SANDBOX_EXEC)
                .args(["-p", p, "/bin/sh", "-c", script])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).into_owned()
        });
        for (name, attack) in [
            (
                "file",
                format!("rm {h}/.claude.json && ln -s {h}/.zshrc {h}/.claude.json"),
            ),
            (
                "folder",
                format!("rm -rf {h}/.claude && ln -s {h} {h}/.claude"),
            ),
            (
                "on-the-way",
                format!("rm -rf {h}/.claude/a && ln -s {h}/secret {h}/.claude/a"),
            ),
            (
                "git-refs",
                format!("rm -rf {h}/repo/.git/refs && ln -s {h} {h}/repo/.git/refs"),
            ),
        ] {
            // The confined agent can make the link (the escape #545 describes)...
            let made = run(&p, &format!("{attack} && echo made"));
            assert!(made.contains("made"), "{name}: {made}");
            // ...and its next start is refused instead of given where it points.
            let e = super::profile(&sb).unwrap_err();
            assert_eq!(
                crate::sandbox_refusal(&e),
                Some(crate::SandboxRefusal::WritableIsLink),
                "{name}"
            );
            // Put back for the next case.
            let _ = std::fs::remove_dir_all(&u);
            for d in [".claude/a", "repo/.git/refs", "secret"] {
                std::fs::create_dir_all(u.join(d)).unwrap();
            }
            std::fs::write(u.join(".claude.json"), "{}").unwrap();
            std::fs::write(u.join(".zshrc"), "# rc\n").unwrap();
            std::fs::write(u.join(".claude/a/b.json"), "{}").unwrap();
            assert!(super::profile(&sb).is_ok(), "{name}: restored");
        }
        // An owner's own link outside what the agent may write (a dotfiles `.gitconfig`) is followed.
        std::os::unix::fs::symlink(u.join(".zshrc"), u.join(".gitconfig")).unwrap();
        let dotfiles = crate::Sandbox {
            read: vec![u.join(".gitconfig")],
            write: vec![u.join(".claude"), "/dev".into()],
            ..Default::default()
        };
        assert!(super::profile(&dotfiles).is_ok());
        // The same link where the agent may write (here all of `/tmp`) could be its own: refused.
        let mine = crate::Sandbox {
            write: vec!["/tmp".into()],
            ..dotfiles
        };
        assert!(super::profile(&mine).is_err());
        let _ = std::fs::remove_dir_all(&u);
    }

    #[test]
    fn a_link_swapped_in_while_the_profile_is_built_never_opens_what_it_points_to() {
        // Review of #555 (1): another agent with the same write entry flips it between a folder and a link to the user's home while
        // this one starts. Checking and resolving are one walk, so every profile either refuses or names the folder itself.
        let u = std::path::PathBuf::from(format!("/tmp/agora-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&u);
        std::fs::create_dir_all(u.join(".claude")).unwrap();
        let real = std::fs::canonicalize(&u).unwrap();
        let sb = crate::Sandbox {
            write: vec![u.join(".claude"), "/dev".into()],
            ..Default::default()
        };
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flipper = {
            let (u, stop) = (u.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut flips = 0u64;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = std::fs::remove_dir(u.join(".claude"));
                    let _ = std::os::unix::fs::symlink(&u, u.join(".claude"));
                    let _ = std::fs::remove_file(u.join(".claude"));
                    let _ = std::fs::create_dir(u.join(".claude"));
                    flips += 1;
                }
                flips
            })
        };
        let home_rule = format!("(subpath \"{}\")", real.display());
        let (mut ok, mut refused) = (0, 0);
        for _ in 0..3000 {
            match super::profile(&sb) {
                Ok(p) => {
                    assert!(!p.contains(&home_rule), "{p}");
                    ok += 1;
                }
                Err(_) => refused += 1,
            }
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let flips = flipper.join().unwrap();
        assert!(flips > 100, "the link was swapped {flips} times");
        eprintln!("race: {flips} swaps, {ok} profiles, {refused} refused");
        let _ = std::fs::remove_dir_all(&u);
    }

    #[test]
    fn another_spelling_of_a_place_the_agent_may_write_is_the_same_place() {
        // Review of #555 (2): the match was on spelling. Write entries spelled through the `/System/Volumes/Data` firmlink or in
        // other case, and a link the agent made below the real folder, are still the agent's; and every rule names the kernel's path
        // (a rule spelled through the firmlink matches nothing).
        let u = std::path::PathBuf::from(format!("/tmp/agora-spell-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&u);
        std::fs::create_dir_all(u.join("w")).unwrap();
        std::fs::write(u.join(".zshrc"), "# rc\n").unwrap();
        let real = std::fs::canonicalize(&u).unwrap();
        let firm =
            std::path::Path::new("/System/Volumes/Data").join(real.strip_prefix("/").unwrap());
        let upper = std::path::PathBuf::from(real.display().to_string().to_uppercase());
        for spelled in [firm.join("w"), upper.join("W")] {
            let sb = crate::Sandbox {
                write: vec![spelled.clone(), "/dev".into()],
                deny: vec![firm.join("w/closed")],
                ..Default::default()
            };
            let p = super::profile(&sb).unwrap();
            assert!(
                p.contains(&format!("(subpath \"{}\")", real.join("w").display())),
                "{spelled:?}: {p}"
            );
            assert!(
                p.contains(&format!(
                    "(subpath \"{}\")",
                    real.join("w/closed").display()
                )),
                "{spelled:?}: {p}"
            );
            assert!(!p.contains("/System/Volumes/Data"), "{p}");
            // A link the agent made below it, read through an entry spelled the kernel's way: refused.
            std::os::unix::fs::symlink(u.join(".zshrc"), u.join("w/rc")).unwrap();
            let via = crate::Sandbox {
                read: vec![real.join("w/rc")],
                ..sb
            };
            assert_eq!(
                crate::sandbox_refusal(&super::profile(&via).unwrap_err()),
                Some(crate::SandboxRefusal::WritableIsLink),
                "{spelled:?}"
            );
            std::fs::remove_file(u.join("w/rc")).unwrap();
        }
        let _ = std::fs::remove_dir_all(&u);
    }

    #[test]
    fn a_file_it_may_write_that_has_another_name_refuses_the_start() {
        // #571: a hard link made outside confinement (measured: writing through it changed the other file). A file entry with a
        // second name is refused; the same file with one name starts, and so does a folder entry holding such a file (accepted risk).
        let u = std::path::PathBuf::from(format!("/tmp/agora-571-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&u);
        std::fs::create_dir_all(u.join("w")).unwrap();
        std::fs::write(u.join(".zshrc"), "# rc\n").unwrap();
        std::fs::write(u.join(".claude.json"), "{}").unwrap();
        let sb = crate::Sandbox {
            write: vec![u.join(".claude.json"), u.join("w"), "/dev".into()],
            ..Default::default()
        };
        assert!(super::profile(&sb).is_ok());
        std::fs::remove_file(u.join(".claude.json")).unwrap();
        std::fs::hard_link(u.join(".zshrc"), u.join(".claude.json")).unwrap();
        assert_eq!(
            crate::sandbox_refusal(&super::profile(&sb).unwrap_err()),
            Some(crate::SandboxRefusal::WritableHasLinks)
        );
        std::fs::remove_file(u.join(".claude.json")).unwrap();
        std::fs::write(u.join(".claude.json"), "{}").unwrap();
        // A save sibling it may write (`.claude.json.tmp`) that is already a second name of `.zshrc` (review of #592): refused,
        // and Seatbelt would have let the write through (measured with this file before the fix).
        std::fs::hard_link(u.join(".zshrc"), u.join(".claude.json.tmp")).unwrap();
        assert_eq!(
            crate::sandbox_refusal(&super::profile(&sb).unwrap_err()),
            Some(crate::SandboxRefusal::WritableHasLinks)
        );
        std::fs::remove_file(u.join(".claude.json.tmp")).unwrap();
        // A sibling with one name, and a link whose name only looks alike (`.claude.jsonx`), are fine.
        std::fs::write(u.join(".claude.json.lock"), "").unwrap();
        std::fs::hard_link(u.join(".zshrc"), u.join(".claude.jsonx")).unwrap();
        assert!(super::profile(&sb).is_ok());
        std::fs::hard_link(u.join(".zshrc"), u.join("w/rc")).unwrap();
        assert!(super::profile(&sb).is_ok(), "folders are not scanned");
        let _ = std::fs::remove_dir_all(&u);
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
            let e = super::profile(&sb).unwrap_err();
            assert_eq!(
                crate::sandbox_refusal(&e),
                Some(crate::SandboxRefusal::ProtectedPathUnresolved),
                "{deny:?}"
            );
            assert!(
                super::wrap(&sb, "/bin/sh".as_ref(), &[]).is_err(),
                "{deny:?}"
            );
        }
    }

    #[test]
    fn preferences_follow_the_file_rules_and_are_not_a_way_around_them() {
        // `defaults` goes through cfprefsd, which reads and writes `~/Library/Preferences` itself (F201).
        struct Domain(String);
        impl Drop for Domain {
            fn drop(&mut self) {
                let _ = std::process::Command::new("/usr/bin/defaults")
                    .args(["delete", &self.0])
                    .output();
                if let Some(home) = std::env::var_os("HOME") {
                    let plist = format!("Library/Preferences/{}.plist", self.0);
                    let _ = std::fs::remove_file(std::path::Path::new(&home).join(plist));
                }
            }
        }
        let domain = Domain(format!("dev.agora.test.prefs.{}", std::process::id()));
        let defaults = |sb: Option<&crate::Sandbox>, args: &[&str]| {
            let p = sb.map(|sb| super::profile(sb).unwrap());
            let mut cmd = std::process::Command::new(match &p {
                Some(_) => super::SANDBOX_EXEC,
                None => "/usr/bin/defaults",
            });
            if let Some(p) = &p {
                cmd.args(["-p", p, "/usr/bin/defaults"]);
            }
            let out = cmd.args(args).output().unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        defaults(None, &["write", &domain.0, "token", "secret"]);
        let sb = crate::Sandbox {
            read: vec!["/usr".into(), "/bin".into()],
            write: vec!["/dev".into()],
            ..Default::default()
        };
        assert_eq!(defaults(Some(&sb), &["read", &domain.0, "token"]), "");
        defaults(Some(&sb), &["write", &domain.0, "token", "changed"]);
        assert_eq!(defaults(None, &["read", &domain.0, "token"]), "secret");
        // The global domain (language, locale) stays readable.
        assert_eq!(
            defaults(Some(&sb), &["read", "-g", "AppleLocale"]),
            defaults(None, &["read", "-g", "AppleLocale"])
        );
    }

    #[test]
    fn confined_sockets_name_the_allow_list_and_the_nodes_closed_folders_come_after() {
        // ADR-0083. Text only; the run below measures what Seatbelt does with it.
        let sb = crate::Sandbox {
            read: vec!["/usr".into()],
            write: vec![std::env::temp_dir()],
            sockets: vec![std::env::temp_dir().join("agora-sock-text")],
            deny: vec![std::env::temp_dir().join("agora-sock-text-home")],
            confine_sockets: true,
            ..Default::default()
        };
        let p = super::profile(&sb).unwrap();
        let general = p
            .find("(deny network-outbound (remote unix-socket (subpath \"/\")))")
            .unwrap_or(usize::MAX);
        assert!(
            general != usize::MAX,
            "every unix socket is refused first: {p}"
        );
        let resolver = p
            .find("/private/var/run/mDNSResponder")
            .unwrap_or(usize::MAX);
        // The node's closed folder: the last such deny, after the allow-list (a `quote`d filter, one form, never nested).
        let closed = p
            .rfind("(deny network-outbound (remote unix-socket (subpath \"")
            .unwrap_or(usize::MAX);
        assert!(
            general < resolver && resolver < closed && closed != usize::MAX,
            "{p}"
        );
        assert!(!p.contains("(subpath (subpath"), "{p}");
        assert!(!p.contains("(deny network*)"), "{p}");
    }

    /// Every profile shape this module can produce must PARSE: a profile `sandbox-exec` refuses (exit 65) means the agent never starts,
    /// which text assertions cannot see (the nested `subpath` of ADR-0083's first version passed them all).
    #[test]
    fn every_profile_shape_parses_in_sandbox_exec() {
        let base = std::path::PathBuf::from(format!("/tmp/agora-parse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let home = base.join("home");
        let tree = home.join("worktrees/w");
        let sock = home.join("mesh/agent.sock");
        let owner = base.join("owner.sock");
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
        let mut ran = 0;
        for offline in [false, true] {
            for confine_sockets in [false, true] {
                for with_owner in [false, true] {
                    for node_folders in [false, true] {
                        let mut sb = crate::Sandbox {
                            read: vec!["/usr".into(), "/bin".into()],
                            write: vec![tree.clone(), "/tmp".into()],
                            offline,
                            confine_sockets,
                            sockets: vec![sock.clone()],
                            ..Default::default()
                        };
                        if with_owner {
                            sb.sockets.push(owner.clone());
                        }
                        if node_folders {
                            sb.deny = vec![home.clone()];
                            sb.reopen = vec![tree.clone()];
                        }
                        let p = super::profile(&sb).unwrap();
                        let out = std::process::Command::new(super::SANDBOX_EXEC)
                            .args(["-p", &p, "/usr/bin/true"])
                            .output()
                            .unwrap();
                        assert!(
                            out.status.success(),
                            "offline={offline} confine_sockets={confine_sockets} owner={with_owner} node_folders={node_folders}: \
                             {:?} {}\n{p}",
                            out.status,
                            String::from_utf8_lossy(&out.stderr)
                        );
                        ran += 1;
                    }
                }
            }
        }
        assert_eq!(ran, 16);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_confined_process_reaches_only_the_sockets_it_is_given() {
        use std::os::unix::net::UnixListener;
        let dir = std::path::PathBuf::from(format!("/tmp/agora-sk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["own", "other"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        let own = UnixListener::bind(dir.join("own/mesh")).unwrap();
        let other = UnixListener::bind(dir.join("other/agent.1")).unwrap();
        let mut sb = crate::Sandbox {
            read: vec!["/usr".into(), "/bin".into()],
            write: vec![dir.join("own"), "/tmp".into(), "/dev".into()],
            sockets: vec![dir.join("own")],
            confine_sockets: true,
            ..Default::default()
        };
        let d = dir.display();
        let script = format!(
            "nc -U -w 1 {d}/own/mesh </dev/null 2>/dev/null && echo own-open || echo own-closed; \
             nc -U -w 1 {d}/other/agent.1 </dev/null 2>/dev/null && echo other-open || echo other-closed; \
             nc -z -G 2 127.0.0.1 1 2>/dev/null; echo tcp-not-blocked-by-sockets; \
             /usr/bin/dscacheutil -q host -a name localhost 2>/dev/null | grep -q 127.0.0.1 && echo resolver-open || echo resolver-closed;"
        );
        let run = |sb: &crate::Sandbox| {
            let p = super::profile(sb).unwrap();
            let out = std::process::Command::new(super::SANDBOX_EXEC)
                .args(["-p", &p, "/bin/sh", "-c", &script])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        let text = run(&sb);
        for want in ["own-open", "other-closed", "resolver-open"] {
            assert!(text.contains(want), "{want}: {text}");
        }
        // Negative control: the same profile without the restriction reaches the other socket.
        sb.confine_sockets = false;
        let text = run(&sb);
        assert!(text.contains("other-open"), "{text}");
        drop((own, other));
        let _ = std::fs::remove_dir_all(dir);
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
