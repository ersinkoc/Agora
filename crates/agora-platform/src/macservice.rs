//! macOS half of `agora service` (ADR-0062 item 6): a LaunchAgent for the current user, in `~/Library/LaunchAgents`, loaded by
//! launchd into the user's login session (`gui/<uid>`). No administrator rights and nothing outside the user's own folder.
//! - `install` writes the job's file (owner-only, renamed into place, never through a planted link) and, when the job was turned off
//!   (`launchctl disable`, Login Items), turns it on again; launchd reads the file at the next login. `start` loads it now
//!   (`launchctl bootstrap`, or `kickstart` when it is loaded already).
//! - Every file Agora writes starts with [`MARKER`]. A file at the job's place without it (a LaunchAgent made by hand or by another
//!   tool), a link or a folder there, or a job of the same label that launchd loaded from another file, is never replaced, stopped
//!   nor removed: [`crate::ServiceProblem::NotOurs`] / [`crate::ServiceProblem::InTheWay`].
//! - `RunAtLoad`: it starts at login. `KeepAlive = { SuccessfulExit = false }` when asked: launchd starts the supervisor again after
//!   it failed (a non-zero exit, or killed by any signal: `kill -9` and the out-of-memory killer too). Under a service the supervisor
//!   exits 0 for every end it decides itself (a give-up, a refusal, a clean stop), so those are not restarted. launchd waits its
//!   default 10 s (`ThrottleInterval`) between starts.
//! - `AbandonProcessGroup` stays false (launchd's default): when the job is unloaded, what it started is ended with it, so nothing
//!   an agent left behind outlives the service.
//! - `remove` unloads it (`launchctl bootout`) and deletes the file. The caller stops the role through its lock first.
//! - Refused as root ([`crate::ServiceProblem::Root`]): `sudo` would write root's job, or root's file into the user's folder.
//! - Nothing is written to standard output or error: the supervisor writes its own log (`--log`).
//!
//! `launchctl` is run by its full path, never looked up on `PATH`. Every value in the file is XML-escaped; a value a property list
//! cannot carry (not Unicode, a control character) and a program or folder that is not a full path are refused.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::{ServiceProblem, ServiceRefusal, ServiceSpec, ServiceState};

const LAUNCHCTL: &str = "/bin/launchctl";

/// The first two lines of every job file Agora writes. A file at the same place without them is not Agora's.
const MARKER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
    <!-- Login service (ADR-0062 item 6): written and removed by its own service command; edits are not kept. -->\n";

fn uid() -> u32 {
    // SAFETY: getuid cannot fail and has no preconditions.
    unsafe { libc::getuid() }
}

/// `gui/<uid>`: this user's login session, where LaunchAgents run.
fn domain() -> String {
    format!("gui/{}", uid())
}

fn target(name: &str) -> String {
    format!("{}/{name}", domain())
}

/// This user's home folder, from the user database (not `HOME`, which a caller's environment could point elsewhere).
fn home() -> io::Result<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    // SAFETY: getpwuid returns a pointer into static storage or null; the fields are read at once, before any other call.
    unsafe {
        let pw = libc::getpwuid(uid());
        if pw.is_null() || (*pw).pw_dir.is_null() {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        let dir = std::ffi::CStr::from_ptr((*pw).pw_dir);
        Ok(PathBuf::from(OsStr::from_bytes(dir.to_bytes())))
    }
}

/// See [`crate::service_precheck`]: not as root (effective or real).
pub(crate) fn precheck() -> io::Result<()> {
    // SAFETY: plain call.
    if unsafe { libc::geteuid() } == 0 || uid() == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            ServiceProblem::Root,
        ));
    }
    Ok(())
}

/// Where the job's file is: `~/Library/LaunchAgents/<name>.plist`.
fn plist_path(name: &str) -> io::Result<PathBuf> {
    valid_name(name)?;
    Ok(home()?
        .join("Library/LaunchAgents")
        .join(format!("{name}.plist")))
}

/// A label launchd and a file name both carry unchanged.
fn valid_name(name: &str) -> io::Result<()> {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !name.starts_with('.');
    if ok {
        Ok(())
    } else {
        Err(io::Error::from(io::ErrorKind::InvalidInput))
    }
}

fn launchctl(args: &[&str]) -> io::Result<Output> {
    Command::new(LAUNCHCTL)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
}

/// What `launchctl` said when it failed, passed on as the reason (never parsed).
fn failed(out: &Output) -> io::Error {
    let said = String::from_utf8_lossy(if out.stderr.is_empty() {
        &out.stdout
    } else {
        &out.stderr
    })
    .trim()
    .to_string();
    if said.is_empty() {
        return io::Error::other(format!("launchctl: {}", out.status));
    }
    io::Error::other(said)
}

fn checked(args: &[&str]) -> io::Result<()> {
    let out = launchctl(args)?;
    if out.status.success() {
        Ok(())
    } else {
        Err(failed(&out))
    }
}

fn not_ours(path: PathBuf) -> io::Error {
    io::Error::new(io::ErrorKind::AlreadyExists, ServiceProblem::NotOurs(path))
}

fn in_the_way(path: PathBuf) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, ServiceProblem::InTheWay(path))
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// Why `s` cannot go into a property list as it is: not valid Unicode, or a control character (XML 1.0 cannot carry most of them,
/// and a line break in a path is never meant).
fn carry(s: &OsStr) -> Result<&str, ServiceRefusal> {
    let t = s.to_str().ok_or(ServiceRefusal::CannotCarry)?;
    if t.chars().any(char::is_control) {
        return Err(ServiceRefusal::ControlCharacter);
    }
    Ok(t)
}

/// See [`crate::service_refusal`]: every value must be Unicode without control characters; the program and the working directory
/// must be full paths.
pub(crate) fn refusal(spec: &ServiceSpec) -> Option<(std::ffi::OsString, ServiceRefusal)> {
    for p in [&spec.program, &spec.work_dir] {
        if !p.is_absolute() {
            return Some((p.as_os_str().to_owned(), ServiceRefusal::NotAbsolute));
        }
    }
    std::iter::once(spec.program.as_os_str())
        .chain(spec.args.iter().map(std::ffi::OsString::as_os_str))
        .chain(std::iter::once(spec.work_dir.as_os_str()))
        .chain(spec.env.iter().map(|(_, v)| v.as_os_str()))
        .find_map(|a| carry(a).err().map(|why| (a.to_owned(), why)))
}

fn text(s: &OsStr) -> io::Result<String> {
    carry(s)
        .map(xml_escape)
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

/// The job's property list for `spec`.
pub(crate) fn plist(spec: &ServiceSpec) -> io::Result<String> {
    valid_name(&spec.name)?;
    if refusal(spec).is_some() {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let mut args = format!("\t\t<string>{}</string>\n", text(spec.program.as_os_str())?);
    for a in &spec.args {
        args.push_str(&format!("\t\t<string>{}</string>\n", text(a)?));
    }
    let mut env = String::new();
    for (k, v) in &spec.env {
        env.push_str(&format!(
            "\t\t<key>{}</key>\n\t\t<string>{}</string>\n",
            text(OsStr::new(k))?,
            text(v)?
        ));
    }
    let keep_alive = if spec.restart_supervisor_on_failure {
        "\t<key>KeepAlive</key>\n\t<dict>\n\t\t<key>SuccessfulExit</key>\n\t\t<false/>\n\t</dict>\n"
    } else {
        "\t<key>KeepAlive</key>\n\t<false/>\n"
    };
    Ok(format!(
        "{MARKER}\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         \t<key>Label</key>\n\t<string>{label}</string>\n\
         \t<key>Comment</key>\n\t<string>{comment}</string>\n\
         \t<key>ProgramArguments</key>\n\t<array>\n{args}\t</array>\n\
         \t<key>WorkingDirectory</key>\n\t<string>{dir}</string>\n\
         \t<key>EnvironmentVariables</key>\n\t<dict>\n{env}\t</dict>\n\
         \t<key>RunAtLoad</key>\n\t<true/>\n\
         {keep_alive}\
         \t<key>AbandonProcessGroup</key>\n\t<false/>\n\
         \t<key>ProcessType</key>\n\t<string>Standard</string>\n\
         </dict>\n</plist>\n",
        label = xml_escape(&spec.name),
        comment = text(OsStr::new(&spec.description))?,
        dir = text(spec.work_dir.as_os_str())?,
    ))
}

/// What is at the job file's place: nothing, Agora's own file (with its text), or something Agora must leave alone. A link is never
/// followed: the file is opened with `O_NOFOLLOW` after it was seen to be a plain file.
enum AtPlace {
    Absent,
    Ours(Vec<u8>),
    /// A plain file without the marker.
    Theirs,
    /// A link, a folder or anything but a plain file.
    InTheWay,
}

fn at_place(path: &Path) -> io::Result<AtPlace> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    match fs::symlink_metadata(path) {
        Ok(m) if !m.is_file() => return Ok(AtPlace::InTheWay),
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(AtPlace::Absent),
        Err(e) => return Err(e),
    }
    let mut f = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => return Ok(AtPlace::InTheWay),
        Err(e) => return Err(e),
    };
    if !f.metadata()?.is_file() {
        return Ok(AtPlace::InTheWay);
    }
    let mut was = Vec::new();
    f.read_to_end(&mut was)?;
    Ok(if was.starts_with(MARKER.as_bytes()) {
        AtPlace::Ours(was)
    } else {
        AtPlace::Theirs
    })
}

/// The file launchd loaded the job `name` from, when it has it loaded in this user's session (`path = ...` of `launchctl print`).
/// `Some(None)` when it is loaded but `print` names no file.
fn loaded_from(name: &str) -> io::Result<Option<Option<PathBuf>>> {
    let out = launchctl(&["print", &target(name)])?;
    if !out.status.success() {
        return Ok(None);
    }
    Ok(Some(read_print_path(&String::from_utf8_lossy(&out.stdout))))
}

/// The job's own `path = ...` line of `launchctl print` (at one tab: nested blocks are indented further).
pub(crate) fn read_print_path(listing: &str) -> Option<PathBuf> {
    listing.lines().find_map(|l| {
        let rest = l.strip_prefix('\t')?;
        if rest.starts_with('\t') {
            return None;
        }
        rest.strip_prefix("path = ").map(PathBuf::from)
    })
}

/// Whether launchd has the job loaded from a file other than `ours`; that file is named (the label, when `print` names none).
fn loaded_elsewhere(name: &str, ours: &Path) -> io::Result<Option<PathBuf>> {
    let same = |a: &Path| {
        a == ours
            || fs::canonicalize(a)
                .ok()
                .is_some_and(|a| fs::canonicalize(ours).ok() == Some(a))
    };
    Ok(match loaded_from(name)? {
        Some(Some(p)) if !same(&p) => Some(p),
        Some(None) => Some(PathBuf::from(name)),
        _ => None,
    })
}

/// See [`crate::service_foreign`].
pub(crate) fn foreign(name: &str) -> io::Result<Option<PathBuf>> {
    let path = plist_path(name)?;
    match at_place(&path)? {
        AtPlace::Theirs | AtPlace::InTheWay => return Ok(Some(path)),
        AtPlace::Absent | AtPlace::Ours(_) => {}
    }
    loaded_elsewhere(name, &path)
}

pub(crate) fn install(spec: &ServiceSpec) -> io::Result<()> {
    precheck()?;
    let body = plist(spec)?;
    let path = plist_path(&spec.name)?;
    if let Some(theirs) = loaded_elsewhere(&spec.name, &path)? {
        return Err(not_ours(theirs));
    }
    match at_place(&path)? {
        AtPlace::InTheWay => return Err(in_the_way(path)),
        AtPlace::Theirs => return Err(not_ours(path)),
        AtPlace::Ours(was) if was == body.as_bytes() => {}
        AtPlace::Ours(_) | AtPlace::Absent => {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }
            // Written beside it (exclusive, 0600 from its creation) and renamed over it: `rename` replaces the name, never a
            // link's target.
            crate::write_private(&path, body.as_bytes())?;
        }
    }
    // Turned off (`launchctl disable`, Login Items): turned on again, and checked, so `install` never claims a job launchd skips.
    if disabled(&spec.name)? {
        checked(&["enable", &target(&spec.name)])?;
        if disabled(&spec.name)? {
            return Err(io::Error::other(format!(
                "launchctl enable {} did not turn it on",
                target(&spec.name)
            )));
        }
    }
    Ok(())
}

pub(crate) fn start(name: &str) -> io::Result<()> {
    precheck()?;
    let path = plist_path(name)?;
    match at_place(&path)? {
        AtPlace::Ours(_) => {}
        AtPlace::Absent => return Err(io::Error::from(io::ErrorKind::NotFound)),
        AtPlace::Theirs => return Err(not_ours(path)),
        AtPlace::InTheWay => return Err(in_the_way(path)),
    }
    if let Some(theirs) = loaded_elsewhere(name, &path)? {
        return Err(not_ours(theirs));
    }
    if loaded_from(name)?.is_some() {
        return checked(&["kickstart", &target(name)]);
    }
    let p = path
        .to_str()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    checked(&["bootstrap", &domain(), p])
}

pub(crate) fn remove(name: &str) -> io::Result<bool> {
    precheck()?;
    let path = plist_path(name)?;
    let place = at_place(&path)?;
    // Only a job Agora wrote is unloaded and deleted; one made by hand under the same label is left exactly as it is.
    match place {
        AtPlace::Theirs => return Err(not_ours(path)),
        AtPlace::InTheWay => return Err(in_the_way(path)),
        AtPlace::Absent | AtPlace::Ours(_) => {}
    }
    if let Some(theirs) = loaded_elsewhere(name, &path)? {
        return Err(not_ours(theirs));
    }
    let was_loaded = loaded_from(name)?.is_some();
    if was_loaded {
        // Ends what it still runs too (the caller stopped the role through its lock already).
        checked(&["bootout", &target(name)])?;
    }
    if matches!(place, AtPlace::Absent) {
        return Ok(was_loaded);
    }
    match fs::symlink_metadata(&path) {
        Ok(m) if m.is_file() => fs::remove_file(&path)?,
        Ok(_) => return Err(in_the_way(path)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    Ok(true)
}

/// Whether `name` is turned off for this user (`launchctl disable`, or Login Items in System Settings), from
/// `launchctl print-disabled`'s `"<label>" => disabled` (or `=> true` on older systems) lines.
pub(crate) fn disabled_in(listing: &str, name: &str) -> bool {
    let quoted = format!("\"{name}\"");
    listing.lines().any(|l| {
        let l = l.trim();
        l.starts_with(&quoted)
            && l[quoted.len()..]
                .trim_start()
                .strip_prefix("=>")
                .is_some_and(|v| matches!(v.trim(), "disabled" | "true"))
    })
}

fn disabled(name: &str) -> io::Result<bool> {
    let out = launchctl(&["print-disabled", &domain()])?;
    if !out.status.success() {
        return Err(failed(&out));
    }
    Ok(disabled_in(&String::from_utf8_lossy(&out.stdout), name))
}

/// Registered when a file is at the job's place (Agora's or not: the caller asks [`foreign`]) or launchd has the job loaded.
pub(crate) fn status(name: &str) -> io::Result<ServiceState> {
    let path = plist_path(name)?;
    let file = !matches!(at_place(&path)?, AtPlace::Absent);
    if !file && loaded_from(name)?.is_none() {
        return Ok(ServiceState::NotInstalled);
    }
    Ok(ServiceState::Installed {
        enabled: !disabled(name)?,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn spec() -> ServiceSpec {
        ServiceSpec {
            name: "agora-node".into(),
            description: "Agora node <for you> & me".into(),
            program: "/Users/a b/.local/bin/agora".into(),
            args: vec![
                "supervise".into(),
                "node".into(),
                "--home".into(),
                "/Users/a&b <c>/.agora".into(),
                "--log".into(),
            ],
            env: vec![("PATH".into(), "/usr/bin:/bin:/Users/a b/.cargo/bin".into())],
            work_dir: "/Users/a&b <c>/.agora".into(),
            restart_supervisor_on_failure: true,
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agora-macsvc-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn lint(p: &str) -> bool {
        let d = scratch("lint");
        let f = d.join("job.plist");
        fs::write(&f, p).unwrap();
        let ok = Command::new("/usr/bin/plutil")
            .args(["-lint", "-s"])
            .arg(&f)
            .status()
            .unwrap()
            .success();
        let _ = fs::remove_dir_all(&d);
        ok
    }

    #[test]
    fn the_job_runs_the_supervisor_at_login_and_is_restarted_only_after_it_failed() {
        let p = plist(&spec()).unwrap();
        assert!(p.starts_with(MARKER), "{p}");
        for want in [
            "<key>Label</key>\n\t<string>agora-node</string>",
            "<string>/Users/a b/.local/bin/agora</string>\n\t\t<string>supervise</string>\n\t\t<string>node</string>",
            "<string>--home</string>\n\t\t<string>/Users/a&amp;b &lt;c&gt;/.agora</string>\n\t\t<string>--log</string>",
            "<key>WorkingDirectory</key>\n\t<string>/Users/a&amp;b &lt;c&gt;/.agora</string>",
            "<key>PATH</key>\n\t\t<string>/usr/bin:/bin:/Users/a b/.cargo/bin</string>",
            "<key>RunAtLoad</key>\n\t<true/>",
            "<key>KeepAlive</key>\n\t<dict>\n\t\t<key>SuccessfulExit</key>\n\t\t<false/>",
            "<key>AbandonProcessGroup</key>\n\t<false/>",
            "Agora node &lt;for you&gt; &amp; me",
        ] {
            assert!(p.contains(want), "{want}\n{p}");
        }
        // No log redirection: the supervisor writes its own.
        assert!(!p.contains("StandardOutPath") && !p.contains("StandardErrorPath"));
        let mut s = spec();
        s.restart_supervisor_on_failure = false;
        let off = plist(&s).unwrap();
        assert!(off.contains("<key>KeepAlive</key>\n\t<false/>"));
        // `plutil` agrees both are property lists, the marker comment included, and reads the escaped folder back unchanged.
        assert!(lint(&p), "{p}");
        assert!(lint(&off), "{off}");
        let d = scratch("read");
        let f = d.join("job.plist");
        fs::write(&f, &p).unwrap();
        let out = Command::new("/usr/bin/plutil")
            .args(["-extract", "WorkingDirectory", "raw", "-o", "-"])
            .arg(&f)
            .output()
            .unwrap();
        let _ = fs::remove_dir_all(&d);
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "/Users/a&b <c>/.agora"
        );
    }

    #[test]
    fn a_value_the_file_cannot_carry_or_a_name_launchd_would_change_is_refused() {
        let mut s = spec();
        s.args.push("a\nb".into());
        assert_eq!(
            refusal(&s).map(|(_, w)| w),
            Some(ServiceRefusal::ControlCharacter)
        );
        assert_eq!(plist(&s).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        let mut s = spec();
        s.work_dir = "relative/.agora".into();
        assert_eq!(
            refusal(&s).map(|(_, w)| w),
            Some(ServiceRefusal::NotAbsolute)
        );
        let mut s = spec();
        s.program = "agora".into();
        assert_eq!(
            refusal(&s).map(|(_, w)| w),
            Some(ServiceRefusal::NotAbsolute)
        );
        for name in ["", "../x", "a/b", ".hidden", "a b"] {
            let mut s = spec();
            s.name = name.into();
            assert_eq!(
                plist(&s).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{name:?}"
            );
        }
        // Not valid Unicode.
        use std::os::unix::ffi::OsStringExt;
        let mut s = spec();
        s.program = PathBuf::from(std::ffi::OsString::from_vec(vec![b'/', 0xff]));
        assert_eq!(
            refusal(&s).map(|(_, w)| w),
            Some(ServiceRefusal::CannotCarry)
        );
        assert_eq!(plist(&s).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        // `%` means nothing to launchd: kept.
        let mut s = spec();
        s.args.push("100%".into());
        assert_eq!(refusal(&s), None);
    }

    #[test]
    fn turned_off_is_read_from_the_disabled_list() {
        let listing = "disabled services = {\n\t\"com.apple.x\" => enabled\n\t\"agora-node\" => disabled\n\t\"agora-hub\" => enabled\n}\n";
        assert!(disabled_in(listing, "agora-node"));
        assert!(!disabled_in(listing, "agora-hub"));
        assert!(!disabled_in(listing, "agora"));
        assert!(disabled_in("\t\"agora-node\" => true\n", "agora-node"));
    }

    #[test]
    fn the_loaded_file_is_the_jobs_own_path_line() {
        let listing = "gui/501/agora-node = {\n\tactive count = 1\n\tpath = /Users/a b/Library/LaunchAgents/agora-node.plist\n\
                       \ttype = LaunchAgent\n\tendpoints = {\n\t\t\"x\" = {\n\t\t\tpath = /elsewhere\n\t\t}\n\t}\n}\n";
        assert_eq!(
            read_print_path(listing),
            Some(PathBuf::from(
                "/Users/a b/Library/LaunchAgents/agora-node.plist"
            ))
        );
        assert_eq!(
            read_print_path("gui/501/x = {\n\tstate = running\n}\n"),
            None
        );
    }

    #[test]
    fn only_a_plain_file_with_the_marker_is_agoras() {
        let d = scratch("place");
        let f = d.join("job.plist");
        assert!(matches!(at_place(&f).unwrap(), AtPlace::Absent));
        fs::write(&f, plist(&spec()).unwrap()).unwrap();
        assert!(matches!(at_place(&f).unwrap(), AtPlace::Ours(_)));
        // Made by hand: the same job without the marker.
        let by_hand = plist(&spec())
            .unwrap()
            .replacen(MARKER, "<?xml version=\"1.0\"?>\n", 1);
        fs::write(&f, by_hand).unwrap();
        assert!(matches!(at_place(&f).unwrap(), AtPlace::Theirs));
        // A link to a file that has the marker is still not Agora's file.
        let real = d.join("real.plist");
        fs::write(&real, plist(&spec()).unwrap()).unwrap();
        fs::remove_file(&f).unwrap();
        std::os::unix::fs::symlink(&real, &f).unwrap();
        assert!(matches!(at_place(&f).unwrap(), AtPlace::InTheWay));
        fs::remove_file(&f).unwrap();
        fs::create_dir(&f).unwrap();
        assert!(matches!(at_place(&f).unwrap(), AtPlace::InTheWay));
        let _ = fs::remove_dir_all(&d);
    }

    /// The real launchd, with a scratch label (`agora-handcheck-<pid>`), a scratch folder and `/bin/sh` as the program: install,
    /// start, a `kill -9` restarted, an exit 0 not restarted, a job turned off turned on by `install`, a file made by hand left alone,
    /// remove. It registers a LaunchAgent for a minute, so it runs only when asked: `AGORA_LAUNCHD_HANDCHECK=1 cargo test -p
    /// agora-platform -- --ignored launchd`. launchd keeps a `"<label>" => enabled` line in its override list afterwards.
    #[test]
    #[ignore = "registers a LaunchAgent with launchd"]
    fn launchd_restarts_a_failed_job_but_not_one_that_exited_0_and_leaves_a_hand_made_job_alone() {
        use std::time::{Duration, Instant};
        if std::env::var_os("AGORA_LAUNCHD_HANDCHECK").is_none() {
            return;
        }
        let name = format!("agora-handcheck-{}", std::process::id());
        let d = scratch("launchd");
        let d = fs::canonicalize(&d).unwrap();
        struct Clean(String, PathBuf);
        impl Drop for Clean {
            fn drop(&mut self) {
                let _ = launchctl(&["bootout", &target(&self.0)]);
                if let Ok(p) = plist_path(&self.0) {
                    let _ = fs::remove_file(p);
                }
                let _ = fs::remove_dir_all(&self.1);
            }
        }
        let _clean = Clean(name.clone(), d.clone());
        let starts = d.join("starts");
        let script = format!(
            "echo $$ >> '{}'; if [ -e '{}' ]; then exit 0; fi; exec /bin/sleep 600",
            starts.display(),
            d.join("exit0").display()
        );
        let spec = ServiceSpec {
            name: name.clone(),
            description: "Agora launchd hand check".into(),
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
            work_dir: d.clone(),
            restart_supervisor_on_failure: true,
        };
        let pids = || -> Vec<i32> {
            fs::read_to_string(&starts)
                .unwrap_or_default()
                .lines()
                .filter_map(|l| l.trim().parse().ok())
                .collect()
        };
        let wait_for = |n: usize, within: Duration| {
            let t = Instant::now();
            while pids().len() < n && t.elapsed() < within {
                std::thread::sleep(Duration::from_millis(100));
            }
            pids().len()
        };
        assert_eq!(status(&name).unwrap(), ServiceState::NotInstalled);
        install(&spec).unwrap();
        assert!(matches!(
            at_place(&plist_path(&name).unwrap()).unwrap(),
            AtPlace::Ours(_)
        ));
        assert_eq!(
            status(&name).unwrap(),
            ServiceState::Installed { enabled: true }
        );
        assert_eq!(foreign(&name).unwrap(), None);
        start(&name).unwrap();
        assert_eq!(wait_for(1, Duration::from_secs(5)), 1, "started");
        // Killed by SIGKILL (as by the out-of-memory killer): started again after launchd's throttle.
        let t = Instant::now();
        // SAFETY: plain call on a pid this test's job wrote.
        unsafe { libc::kill(pids()[0], libc::SIGKILL) };
        assert_eq!(
            wait_for(2, Duration::from_secs(15)),
            2,
            "restarted after kill -9"
        );
        eprintln!("restart after SIGKILL: {:?}", t.elapsed());
        // An exit 0 (an end the supervisor decided) is not restarted: killed once more, it starts, exits 0 and stays ended.
        fs::write(d.join("exit0"), "").unwrap();
        // SAFETY: as above.
        unsafe { libc::kill(pids()[1], libc::SIGKILL) };
        assert_eq!(
            wait_for(3, Duration::from_secs(15)),
            3,
            "restarted once more"
        );
        assert_eq!(
            wait_for(4, Duration::from_secs(15)),
            3,
            "an exit 0 is not restarted"
        );
        // Turned off: `status` says so, `install` turns it on again.
        checked(&["disable", &target(&name)]).unwrap();
        assert_eq!(
            status(&name).unwrap(),
            ServiceState::Installed { enabled: false }
        );
        install(&spec).unwrap();
        assert_eq!(
            status(&name).unwrap(),
            ServiceState::Installed { enabled: true }
        );
        assert!(remove(&name).unwrap());
        assert_eq!(status(&name).unwrap(), ServiceState::NotInstalled);
        assert!(!remove(&name).unwrap());
        // A job file of the same label made by hand: neither replaced, started nor removed.
        let path = plist_path(&name).unwrap();
        let by_hand = plist(&spec).unwrap().replacen(
            MARKER,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            1,
        );
        fs::write(&path, &by_hand).unwrap();
        assert_eq!(foreign(&name).unwrap(), Some(path.clone()));
        for e in [
            install(&spec).unwrap_err(),
            start(&name).unwrap_err(),
            remove(&name).unwrap_err(),
        ] {
            assert!(
                matches!(crate::service_problem(&e), Some(ServiceProblem::NotOurs(_))),
                "{e}"
            );
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), by_hand);
        assert!(loaded_from(&name).unwrap().is_none());
        fs::remove_file(&path).unwrap();
    }
}
