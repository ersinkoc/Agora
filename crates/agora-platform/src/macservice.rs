//! macOS half of `agora service` (ADR-0062 item 6): a LaunchAgent for the current user, in `~/Library/LaunchAgents`, loaded by
//! launchd into the user's login session (`gui/<uid>`). No administrator rights and nothing outside the user's own folder.
//! - `install` only writes the job's file (owner-only, renamed into place, never through a planted link). launchd reads it at the
//!   next login; `start` loads it now (`launchctl bootstrap`, or `kickstart` when it is loaded already).
//! - `RunAtLoad`: it starts at login. `KeepAlive = { Crashed = true }` when asked: launchd starts the supervisor again after a crash,
//!   never after it exited (a supervisor that gave up or was stopped by `agora down` stays ended). Measured on macOS 27.2: after
//!   SIGABRT it was back within 1 s; after SIGTERM (a clean stop) and after SIGKILL it was not. launchd counts only crash signals,
//!   and its other choice, `SuccessfulExit = false`, would also restart a supervisor that gave up (it exits with the role's code).
//! - `AbandonProcessGroup` stays false (launchd's default): when the job is unloaded, what it started is ended with it, so nothing
//!   an agent left behind outlives the service.
//! - `remove` unloads it (`launchctl bootout`) and deletes the file. The caller stops the role through its lock first.
//! - Nothing is written to standard output or error: the supervisor writes its own log (`--log`).
//!
//! `launchctl` is run by its full path, never looked up on `PATH`. Every value in the file is XML-escaped.

use std::ffi::OsStr;
use std::io;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use crate::{ServiceSpec, ServiceState};

const LAUNCHCTL: &str = "/bin/launchctl";

fn uid() -> u32 {
    // SAFETY: getuid cannot fail and has no preconditions.
    unsafe { libc::getuid() }
}

/// `gui/<uid>`: this user's login session, where LaunchAgents run.
fn domain() -> String {
    format!("gui/{}", uid())
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
    io::Error::other(said)
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

/// A value as it goes into the file: valid Unicode, and no control character (a property list cannot carry most of them).
fn text(s: &OsStr) -> io::Result<String> {
    let t = s
        .to_str()
        .filter(|t| !t.chars().any(char::is_control))
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    Ok(xml_escape(t))
}

/// The job's property list for `spec`.
pub(crate) fn plist(spec: &ServiceSpec) -> io::Result<String> {
    valid_name(&spec.name)?;
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
    let keep_alive = if spec.restart_supervisor_on_signal {
        "\t<key>KeepAlive</key>\n\t<dict>\n\t\t<key>Crashed</key>\n\t\t<true/>\n\t</dict>\n"
    } else {
        "\t<key>KeepAlive</key>\n\t<false/>\n"
    };
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
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

pub(crate) fn install(spec: &ServiceSpec) -> io::Result<()> {
    let body = plist(spec)?;
    let path = plist_path(&spec.name)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::write_private(&path, body.as_bytes())
}

/// Whether launchd has the job loaded in this user's session now.
fn loaded(name: &str) -> io::Result<bool> {
    Ok(launchctl(&["print", &format!("{}/{name}", domain())])?
        .status
        .success())
}

pub(crate) fn start(name: &str) -> io::Result<()> {
    let path = plist_path(name)?;
    let out = if loaded(name)? {
        launchctl(&["kickstart", &format!("{}/{name}", domain())])?
    } else {
        let p = path
            .to_str()
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        launchctl(&["bootstrap", &domain(), p])?
    };
    if out.status.success() {
        Ok(())
    } else {
        Err(failed(&out))
    }
}

pub(crate) fn remove(name: &str) -> io::Result<bool> {
    let path = plist_path(name)?;
    let was_loaded = loaded(name)?;
    if was_loaded {
        let out = launchctl(&["bootout", &format!("{}/{name}", domain())])?;
        if !out.status.success() {
            return Err(failed(&out));
        }
    }
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(was_loaded),
        Err(e) => Err(e),
    }
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

pub(crate) fn status(name: &str) -> io::Result<ServiceState> {
    let path = plist_path(name)?;
    match std::fs::symlink_metadata(&path) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(ServiceState::NotInstalled),
        Err(e) => return Err(e),
    }
    let out = launchctl(&["print-disabled", &domain()])?;
    if !out.status.success() {
        return Err(failed(&out));
    }
    let enabled = !disabled_in(&String::from_utf8_lossy(&out.stdout), name);
    Ok(ServiceState::Installed { enabled })
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
                "/Users/a b/.agora".into(),
                "--log".into(),
            ],
            env: vec![("PATH".into(), "/usr/bin:/bin:/Users/a b/.cargo/bin".into())],
            work_dir: "/Users/a b/.agora".into(),
            restart_supervisor_on_signal: true,
        }
    }

    #[test]
    fn the_job_runs_the_supervisor_at_login_and_is_restarted_only_after_a_crash() {
        let p = plist(&spec()).unwrap();
        for want in [
            "<key>Label</key>\n\t<string>agora-node</string>",
            "<string>/Users/a b/.local/bin/agora</string>\n\t\t<string>supervise</string>\n\t\t<string>node</string>",
            "<string>--home</string>\n\t\t<string>/Users/a b/.agora</string>\n\t\t<string>--log</string>",
            "<key>WorkingDirectory</key>\n\t<string>/Users/a b/.agora</string>",
            "<key>PATH</key>\n\t\t<string>/usr/bin:/bin:/Users/a b/.cargo/bin</string>",
            "<key>RunAtLoad</key>\n\t<true/>",
            "<key>KeepAlive</key>\n\t<dict>\n\t\t<key>Crashed</key>\n\t\t<true/>",
            "<key>AbandonProcessGroup</key>\n\t<false/>",
            "Agora node &lt;for you&gt; &amp; me",
        ] {
            assert!(p.contains(want), "{want}\n{p}");
        }
        // No log redirection: the supervisor writes its own.
        assert!(!p.contains("StandardOutPath") && !p.contains("StandardErrorPath"));
        let mut s = spec();
        s.restart_supervisor_on_signal = false;
        assert!(plist(&s)
            .unwrap()
            .contains("<key>KeepAlive</key>\n\t<false/>"));
        // `plutil` agrees it is a property list.
        let f = std::env::temp_dir().join(format!("agora-plist-{}.plist", std::process::id()));
        std::fs::write(&f, &p).unwrap();
        let ok = Command::new("/usr/bin/plutil")
            .args(["-lint", "-s"])
            .arg(&f)
            .status()
            .unwrap()
            .success();
        let _ = std::fs::remove_file(&f);
        assert!(ok, "{p}");
    }

    #[test]
    fn a_value_the_file_cannot_carry_or_a_name_launchd_would_change_is_refused() {
        let mut s = spec();
        s.args.push("a\nb".into());
        assert_eq!(plist(&s).unwrap_err().kind(), io::ErrorKind::InvalidInput);
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
        assert_eq!(plist(&s).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn turned_off_is_read_from_the_disabled_list() {
        let listing = "disabled services = {\n\t\"com.apple.x\" => enabled\n\t\"agora-node\" => disabled\n\t\"agora-hub\" => enabled\n}\n";
        assert!(disabled_in(listing, "agora-node"));
        assert!(!disabled_in(listing, "agora-hub"));
        assert!(!disabled_in(listing, "agora"));
        assert!(disabled_in("\t\"agora-node\" => true\n", "agora-node"));
    }
}
