//! Windows half of `agora service` (ADR-0062 item 6): a logon task for the current user, registered with `schtasks` from a task file,
//! with no administrator rights. Measured on Windows 11 (M7):
//! - `schtasks /create /xml` registers a logon task with `InteractiveToken` and `LeastPrivilege` without elevation;
//! - a console program started by a logon task opens a visible window, so the action runs it under `conhost --headless`;
//! - ending the task (`schtasks /end`) ends only that conhost, not what it started: the caller stops the role through its lock
//!   (`agora down`) before removing the task;
//! - Task Scheduler's restart on failure never fires for a program that ends later, and the supervisor restarts the role itself, so
//!   the task restarts nothing.
//!
//! The task file is written owner-only into the Agora folder and deleted after `schtasks` read it. Every value in it is XML-escaped;
//! programs are named by their full path under `%SystemRoot%\System32`, never looked up on `PATH`.

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use crate::{ServiceProblem, ServiceSpec, ServiceState};

/// Written into every task Agora registers (its `<Source>`) and read back before Agora replaces, starts or deletes a task of its name:
/// a task without it (made by hand or by another tool) is left exactly as it is, as the Linux and macOS halves leave a unit or a
/// LaunchAgent without their marker.
const TASK_MARKER: &str =
    "Login service (ADR-0062 item 6): written and removed by its own service command; edits are not kept.";

fn system32(program: &str) -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| OsString::from(r"C:\Windows")))
        .join("System32")
        .join(program)
}

fn schtasks(args: &[&std::ffi::OsStr]) -> io::Result<Output> {
    Command::new(system32("schtasks.exe"))
        .args(args)
        .stdin(Stdio::null())
        .output()
}

/// What `schtasks` said when it failed (its text is in the system's language: passed on as the reason, never parsed).
fn checked(out: Output) -> io::Result<Output> {
    if out.status.success() {
        return Ok(out);
    }
    let said = String::from_utf8_lossy(if out.stderr.is_empty() {
        &out.stdout
    } else {
        &out.stderr
    })
    .trim()
    .to_string();
    Err(io::Error::other(said))
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

/// A path or argument as it goes into the task: valid Unicode, and without `%`, which Task Scheduler would expand as a variable.
fn text(s: &std::ffi::OsStr) -> io::Result<&str> {
    let invalid = || io::Error::from(io::ErrorKind::InvalidInput);
    let t = s.to_str().ok_or_else(invalid)?;
    if t.contains('%') {
        return Err(invalid());
    }
    Ok(t)
}

/// This process's user as the task names it: its security identifier, which also works for Microsoft and Entra accounts whose
/// `USERNAME` is not their account name.
fn current_user() -> io::Result<String> {
    crate::winconfine::current_user_sid_text()
}

/// The task file for `spec`, run at `user`'s logon.
pub(crate) fn task_xml(spec: &ServiceSpec, user: &str) -> io::Result<String> {
    let mut arguments = String::from("--headless ");
    arguments.push_str(&crate::conpty::quote_arg(text(spec.program.as_os_str())?));
    for a in &spec.args {
        arguments.push(' ');
        arguments.push_str(&crate::conpty::quote_arg(text(a)?));
    }
    let conhost = system32("conhost.exe");
    let user = xml_escape(user);
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Source>{marker}</Source>
    <Description>{description}</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <AllowHardTerminate>true</AllowHardTerminate>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <Arguments>{arguments}</Arguments>
      <WorkingDirectory>{dir}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#,
        marker = xml_escape(TASK_MARKER),
        description = xml_escape(&spec.description),
        command = xml_escape(text(conhost.as_os_str())?),
        arguments = xml_escape(&arguments),
        dir = xml_escape(text(spec.work_dir.as_os_str())?),
    ))
}

/// See [`crate::service_install`]: registers only.
pub(crate) fn install(spec: &ServiceSpec) -> io::Result<()> {
    let xml = task_xml(spec, &current_user()?)?;
    // `/create /f` would replace any task of this name: one Agora did not write is refused first.
    if let Some(theirs) = foreign(&spec.name)? {
        return Err(not_ours(theirs));
    }
    // `schtasks` reads a task file in UTF-16 with its byte order mark.
    let mut bytes = vec![0xFF, 0xFE];
    for unit in xml.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    let file = spec.work_dir.join(format!("{}.task.xml", spec.name));
    crate::write_private(&file, &bytes)?;
    let created = schtasks(&[
        "/create".as_ref(),
        "/tn".as_ref(),
        spec.name.as_ref(),
        "/xml".as_ref(),
        file.as_os_str(),
        "/f".as_ref(),
    ]);
    let _ = std::fs::remove_file(&file);
    checked(created?)?;
    Ok(())
}

/// See [`crate::service_start`].
pub(crate) fn start(name: &str) -> io::Result<()> {
    if let Some(theirs) = foreign(name)? {
        return Err(not_ours(theirs));
    }
    checked(schtasks(&["/run".as_ref(), "/tn".as_ref(), name.as_ref()])?)?;
    Ok(())
}

/// Whether a task `name` (in the root folder) is listed, from the task list's first column (task paths are not translated).
fn listed(name: &str) -> io::Result<bool> {
    let out = checked(schtasks(&[
        "/query".as_ref(),
        "/fo".as_ref(),
        "csv".as_ref(),
        "/nh".as_ref(),
    ])?)?;
    let want = format!("\\{name}");
    Ok(String::from_utf8_lossy(&out.stdout).lines().any(|line| {
        line.split(',')
            .next()
            .is_some_and(|f| f.trim().trim_matches('"').eq_ignore_ascii_case(&want))
    }))
}

/// The task file of `name`, `None` when no task of that name is registered. Its tags are not translated (the task listing is).
fn task_file(name: &str) -> io::Result<Option<String>> {
    let out = schtasks(&[
        "/query".as_ref(),
        "/tn".as_ref(),
        name.as_ref(),
        "/xml".as_ref(),
    ])?;
    if !out.status.success() {
        // Not found, or the scheduler could not answer: only the task list tells them apart.
        return if listed(name)? {
            checked(out).map(|_| None)
        } else {
            Ok(None)
        };
    }
    // The listing may come as UTF-16; the tags are ASCII either way.
    Ok(Some(
        String::from_utf8_lossy(&out.stdout)
            .chars()
            .filter(|c| *c != '\0')
            .collect(),
    ))
}

/// See [`crate::service_status`].
pub(crate) fn status(name: &str) -> io::Result<ServiceState> {
    Ok(match task_file(name)? {
        None => ServiceState::NotInstalled,
        Some(xml) => ServiceState::Installed {
            enabled: settings_enabled(&xml),
        },
    })
}

/// See [`crate::service_foreign`]: `Some(\<name>)` (its place in Task Scheduler) when a task of that name is registered without
/// [`TASK_MARKER`].
pub(crate) fn foreign(name: &str) -> io::Result<Option<PathBuf>> {
    Ok(match task_file(name)? {
        Some(xml) if !has_marker(&xml) => Some(PathBuf::from(format!("\\{name}"))),
        _ => None,
    })
}

/// Whether the task file's `<Source>` is exactly Agora's marker.
fn has_marker(xml: &str) -> bool {
    xml.split_once("<Source>")
        .and_then(|(_, rest)| rest.split_once("</Source>"))
        .is_some_and(|(source, _)| source.trim() == xml_escape(TASK_MARKER))
}

fn not_ours(place: PathBuf) -> io::Error {
    io::Error::new(io::ErrorKind::AlreadyExists, ServiceProblem::NotOurs(place))
}

/// Whether the task's `<Settings>` leave it enabled (a turned-off task says `<Enabled>false</Enabled>` there).
fn settings_enabled(xml: &str) -> bool {
    let settings = xml
        .split_once("<Settings>")
        .and_then(|(_, rest)| rest.split_once("</Settings>"))
        .map_or("", |(s, _)| s);
    !settings.contains("<Enabled>false</Enabled>")
}

/// See [`crate::service_remove`].
pub(crate) fn remove(name: &str) -> io::Result<bool> {
    match task_file(name)? {
        None => return Ok(false),
        Some(xml) if !has_marker(&xml) => return Err(not_ours(PathBuf::from(format!("\\{name}")))),
        Some(_) => {}
    }
    checked(schtasks(&[
        "/delete".as_ref(),
        "/tn".as_ref(),
        name.as_ref(),
        "/f".as_ref(),
    ])?)?;
    Ok(true)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn spec() -> ServiceSpec {
        ServiceSpec {
            name: "agora-node".into(),
            description: "Agora node <starts at logon> & more".into(),
            program: PathBuf::from(r"C:\Program Files\A&B\agora.exe"),
            args: vec![
                "supervise".into(),
                "node".into(),
                "--home".into(),
                r"D:\my agora\home".into(),
                "--log".into(),
            ],
            env: Vec::new(),
            work_dir: PathBuf::from(r"D:\my agora\home"),
            restart_supervisor_on_failure: false,
        }
    }

    #[test]
    fn the_task_runs_the_supervisor_hidden_with_every_value_escaped() {
        let xml = task_xml(&spec(), r"WHITE\o'neil").unwrap();
        assert!(xml.contains("<UserId>WHITE\\o&apos;neil</UserId>"), "{xml}");
        assert!(xml
            .contains("<Description>Agora node &lt;starts at logon&gt; &amp; more</Description>"));
        assert!(xml.contains(r"\System32\conhost.exe</Command>"), "{xml}");
        assert!(
            xml.contains(r#"<Arguments>--headless &quot;C:\Program Files\A&amp;B\agora.exe&quot; supervise node --home &quot;D:\my agora\home&quot; --log</Arguments>"#),
            "{xml}"
        );
        assert!(xml.contains(r"<WorkingDirectory>D:\my agora\home</WorkingDirectory>"));
        assert!(xml.contains("<RunLevel>LeastPrivilege</RunLevel>"));
        // No restart on failure: the supervisor restarts the role and its give-up stays a give-up.
        assert!(!xml.contains("RestartOnFailure"));
        // Agora's own, and known as such when it is read back.
        assert!(has_marker(&xml), "{xml}");
    }

    #[test]
    fn a_task_without_agoras_marker_is_not_agoras() {
        let ours = task_xml(&spec(), "S-1-5-21-1").unwrap();
        // As Task Scheduler gives it back: re-indented, the marker on its own line.
        let read_back = ours
            .replace("<Source>", "<Source>\n      ")
            .replace("</Source>", "\n    </Source>");
        assert!(has_marker(&read_back));
        let hand_made = ours.replace(&format!("<Source>{}</Source>", xml_escape(TASK_MARKER)), "");
        assert!(!has_marker(&hand_made));
        let other_source = ours.replace(&xml_escape(TASK_MARKER), "Some other tool");
        assert!(!has_marker(&other_source));
        let prefix_only = ours.replace(
            &xml_escape(TASK_MARKER),
            &format!("{} plus more", xml_escape(TASK_MARKER)),
        );
        assert!(!has_marker(&prefix_only));
    }

    #[test]
    fn a_percent_sign_or_a_non_unicode_path_is_refused() {
        let mut s = spec();
        s.work_dir = PathBuf::from(r"C:\tmp\a%USERNAME%b");
        assert_eq!(
            task_xml(&s, "S-1-5-21-1").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        let mut s = spec();
        s.args.push("%PATH%".into());
        assert!(task_xml(&s, "S-1-5-21-1").is_err());
    }

    #[test]
    fn a_trailing_backslash_and_spaces_survive_the_command_line() {
        let mut s = spec();
        s.args[3] = r"C:\my home\".into();
        let xml = task_xml(&s, "S-1-5-21-1").unwrap();
        // CommandLineToArgvW reads `"C:\my home\\"` back as `C:\my home\`.
        assert!(
            xml.contains(r#"--home &quot;C:\my home\\&quot; --log"#),
            "{xml}"
        );
    }

    #[test]
    fn a_turned_off_task_is_told_apart_from_its_trigger() {
        let on = "<Triggers><LogonTrigger><Enabled>false</Enabled></LogonTrigger></Triggers><Settings><Enabled>true</Enabled></Settings>";
        let off = "<Triggers><LogonTrigger><Enabled>true</Enabled></LogonTrigger></Triggers><Settings><Enabled>false</Enabled></Settings>";
        assert!(settings_enabled(on));
        assert!(!settings_enabled(off));
    }
}
