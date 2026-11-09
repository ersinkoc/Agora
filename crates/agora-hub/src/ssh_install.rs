//! Installing and enrolling a node on an SSH machine (ADR-0034). Everything here is what the owner could type by hand:
//! four steps, each a fixed remote command in which only validated values appear (a name, a port, a version, a target
//! triple from a short list, a one-time code). The remote side is a trait so the whole conversation is tested against a fake
//! that records what it was asked to run.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::ssh::Machine;

/// What one remote command printed and whether it succeeded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteOut {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs one command on a machine, with `stdin` as its standard input.
pub trait Remote {
    fn run(&self, machine: &Machine, command: &str, stdin: &[u8]) -> Result<RemoteOut, String>;
}

/// The system `ssh`, with the same fixed options as the tunnel (ADR-0032): never prompts, never trusts an unknown host key.
pub struct SshRemote;

impl Remote for SshRemote {
    fn run(&self, machine: &Machine, command: &str, stdin: &[u8]) -> Result<RemoteOut, String> {
        let mut child = Command::new(std::env::var_os("AGORA_SSH").unwrap_or_else(|| "ssh".into()))
            .args([
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "StrictHostKeyChecking=yes",
                "--",
                &machine.destination,
                command,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        if let Some(mut s) = child.stdin.take() {
            // A command that does not read its input closes the pipe early; that is not an error here.
            let _ = s.write_all(stdin);
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        Ok(RemoteOut {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// The release target for what `uname -sm` printed; `None` for anything this does not ship a binary for.
pub fn target_for(uname: &str) -> Option<&'static str> {
    let mut f = uname.split_whitespace();
    match (f.next()?, f.next()?, f.next()) {
        ("Linux", "x86_64", None) => Some("x86_64-unknown-linux-gnu"),
        ("Linux", "aarch64" | "arm64", None) => Some("aarch64-unknown-linux-gnu"),
        ("Darwin", "x86_64", None) => Some("x86_64-apple-darwin"),
        ("Darwin", "arm64" | "aarch64", None) => Some("aarch64-apple-darwin"),
        _ => None,
    }
}

fn safe(s: &str, extra: &[char]) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
}

#[derive(Debug, Clone)]
pub struct Options {
    /// The version the hub runs; a node of exactly this version is left alone.
    pub version: String,
    /// Where release archives are published: `<base>/v<version>/agora-<target>` and `.sha256` (and `.sig`).
    pub release_base: String,
    /// The hub's CA certificate (public), given to the machine on stdin.
    pub ca_pem: String,
    /// The one-time enrollment code.
    pub code: String,
    /// The tunnel's port on the machine.
    pub port: u16,
    /// An `allowed_signers` file for `ssh-keygen -Y verify`: the release key the owner pinned.
    pub release_key: Option<String>,
    /// Install a binary that only has a checksum, no signature. The owner has to ask for this by name.
    pub unsigned: bool,
}

/// A step that stopped the install, with what to tell the owner. `done` lists what already happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallError {
    pub step: &'static str,
    pub detail: String,
    pub done: Vec<&'static str>,
}

fn fail(step: &'static str, detail: impl Into<String>, done: &[&'static str]) -> InstallError {
    InstallError {
        step,
        detail: detail.into(),
        done: done.to_vec(),
    }
}

/// What the install ended up doing on the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finish {
    /// The node runs as a user service.
    Service(&'static str),
    /// Neither systemd nor launchd: the command to run under `tmux`.
    Manual(String),
}

const LINUX_UNIT: &str = "mkdir -p \"$HOME/.config/systemd/user\" && cat > \"$HOME/.config/systemd/user/agora-node.service\" <<EOF\n[Unit]\nDescription=Agora node\nAfter=network-online.target\n\n[Service]\nExecStart=$HOME/.local/bin/agora node run\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\nEOF\nsystemctl --user daemon-reload && systemctl --user enable --now agora-node.service";

const MAC_PLIST: &str = "mkdir -p \"$HOME/Library/LaunchAgents\" && cat > \"$HOME/Library/LaunchAgents/dev.agora.node.plist\" <<EOF\n<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>dev.agora.node</string>\n<key>ProgramArguments</key><array><string>$HOME/.local/bin/agora</string><string>node</string><string>run</string></array>\n<key>RunAtLoad</key><true/><key>KeepAlive</key><true/>\n</dict></plist>\nEOF\nlaunchctl unload \"$HOME/Library/LaunchAgents/dev.agora.node.plist\" 2>/dev/null; launchctl load -w \"$HOME/Library/LaunchAgents/dev.agora.node.plist\"";

/// Runs the four steps. `say` is told what each step is before it runs.
pub fn install(
    remote: &dyn Remote,
    machine: &Machine,
    o: &Options,
    say: &mut dyn FnMut(&'static str),
) -> Result<Finish, InstallError> {
    let mut done: Vec<&'static str> = Vec::new();
    if !safe(&o.version, &['.', '-', '+'])
        || !safe(&o.code, &['-'])
        || !safe(&machine.name, &['.', '_', '-'])
    {
        return Err(fail("check", "unsafe value", &done));
    }
    // 1. Check.
    say("check");
    let run = |cmd: &str, stdin: &[u8], step: &'static str, done: &[&'static str]| {
        remote
            .run(machine, cmd, stdin)
            .map_err(|e| fail(step, e, done))
    };
    let u = run("uname -sm", b"", "check", &done)?;
    if !u.ok {
        return Err(fail("check", u.stderr.trim(), &done));
    }
    let Some(target) = target_for(u.stdout.trim()) else {
        return Err(fail(
            "check",
            format!("no release for {}", u.stdout.trim()),
            &done,
        ));
    };
    done.push("check");
    let have = run(
        "\"$HOME/.local/bin/agora\" --version 2>/dev/null",
        b"",
        "check",
        &done,
    )?;
    let current = have.ok && have.stdout.split_whitespace().any(|w| w == o.version);

    // 2. Binary: only a release asset, checked before it is put in place.
    if !current {
        say("binary");
        if o.release_base.is_empty()
            || !safe(&o.release_base.replace(['/', ':'], ""), &['.', '-', '_'])
        {
            return Err(fail(
                "binary",
                "no release to install from; install it by hand",
                &done,
            ));
        }
        if o.release_key.is_none() && !o.unsigned {
            return Err(fail(
                "binary",
                "the release has to be signed: pin its key, or ask for --unsigned and accept a checksum only",
                &done,
            ));
        }
        let url = format!(
            "{}/v{}/agora-{target}",
            o.release_base.trim_end_matches('/'),
            o.version
        );
        let verify_sig = if o.release_key.is_some() {
            "curl -fsSL -o agora.sig \"$U.sig\" && ssh-keygen -Y verify -f allowed -I agora-release -n file -s agora.sig < agora >/dev/null || { echo signature invalid >&2; exit 4; }; "
        } else {
            ""
        };
        let cmd = format!(
            "set -e; U='{url}'; d=$(mktemp -d); cd \"$d\"; cat > allowed; \
             curl -fsSL -o agora \"$U\"; curl -fsSL -o agora.sha256 \"$U.sha256\"; \
             want=$(cut -d' ' -f1 agora.sha256); \
             got=$( (sha256sum agora 2>/dev/null || shasum -a 256 agora) | cut -d' ' -f1); \
             [ \"$want\" = \"$got\" ] || {{ echo checksum mismatch >&2; exit 3; }}; \
             {verify_sig}\
             mkdir -p \"$HOME/.local/bin\"; install -m 755 agora \"$HOME/.local/bin/agora\"; cd /; rm -rf \"$d\""
        );
        let out = run(
            &cmd,
            o.release_key.as_deref().unwrap_or("").as_bytes(),
            "binary",
            &done,
        )?;
        if !out.ok {
            return Err(fail("binary", out.stderr.trim(), &done));
        }
        done.push("binary");
    }

    // 3. Enroll through the tunnel; the CA (public) comes on stdin and the code goes nowhere else.
    say("enroll");
    let cmd = format!(
        "\"$HOME/.local/bin/agora\" node enroll --hub wss://localhost:{} --ca - --code {} --id {}",
        o.port, o.code, machine.name
    );
    let out = run(&cmd, o.ca_pem.as_bytes(), "enroll", &done)?;
    if !out.ok {
        return Err(fail("enroll", out.stderr.trim(), &done));
    }
    done.push("enroll");

    // 4. Run as a user service, or say what to run.
    say("run");
    let kind = run("uname -s", b"", "run", &done)?;
    if !kind.ok {
        return Err(fail("run", kind.stderr.trim(), &done));
    }
    let svc = match kind.stdout.trim() {
        "Linux" => {
            let has = run("command -v systemctl >/dev/null && systemctl --user show-environment >/dev/null 2>&1", b"", "run", &done)?;
            has.ok.then_some(("systemd", LINUX_UNIT))
        }
        "Darwin" => Some(("launchd", MAC_PLIST)),
        _ => None,
    };
    match svc {
        Some((name, script)) => {
            let out = run(script, b"", "run", &done)?;
            if !out.ok {
                return Err(fail("run", out.stderr.trim(), &done));
            }
            Ok(Finish::Service(name))
        }
        None => Ok(Finish::Manual(
            "tmux new -d -s agora-node '\"$HOME/.local/bin/agora\" node run'".to_string(),
        )),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Answers by the command's first words and records everything it was asked.
    struct Fake {
        log: RefCell<Vec<(String, String)>>,
        uname: &'static str,
        version: Option<&'static str>,
        fail_on: Option<&'static str>,
        systemd: bool,
        uname_s_fails: bool,
    }

    impl Fake {
        fn new(uname: &'static str) -> Self {
            Self {
                log: RefCell::new(Vec::new()),
                uname,
                version: None,
                fail_on: None,
                systemd: true,
                uname_s_fails: false,
            }
        }
    }

    impl Remote for Fake {
        fn run(&self, _: &Machine, command: &str, stdin: &[u8]) -> Result<RemoteOut, String> {
            self.log.borrow_mut().push((
                command.to_string(),
                String::from_utf8_lossy(stdin).into_owned(),
            ));
            let ok = |s: &str| {
                Ok(RemoteOut {
                    ok: true,
                    stdout: s.into(),
                    stderr: String::new(),
                })
            };
            let bad = |s: &str| {
                Ok(RemoteOut {
                    ok: false,
                    stdout: String::new(),
                    stderr: s.into(),
                })
            };
            if let Some(f) = self.fail_on {
                if command.contains(f) {
                    return bad("it broke");
                }
            }
            match command {
                "uname -sm" => ok(self.uname),
                "uname -s" if self.uname_s_fails => bad("connection reset"),
                "uname -s" => ok(self.uname.split_whitespace().next().unwrap()),
                c if c.contains("--version") => match self.version {
                    Some(v) => ok(&format!("agora {v}")),
                    None => bad(""),
                },
                c if c.starts_with("command -v systemctl") => {
                    if self.systemd {
                        ok("")
                    } else {
                        bad("")
                    }
                }
                _ => ok(""),
            }
        }
    }

    fn machine() -> Machine {
        Machine::new("vps", "me@vps", 17443).unwrap()
    }

    fn opts() -> Options {
        Options {
            version: "0.3.0".into(),
            release_base: "https://example.test/releases/download".into(),
            ca_pem: "CA-PEM".into(),
            code: "ABCD-2345".into(),
            port: 17443,
            release_key: Some("allowed-signers".into()),
            unsigned: false,
        }
    }

    #[test]
    fn targets_come_from_a_short_list() {
        assert_eq!(target_for("Linux x86_64"), Some("x86_64-unknown-linux-gnu"));
        assert_eq!(target_for("Darwin arm64"), Some("aarch64-apple-darwin"));
        assert_eq!(target_for("Linux riscv64"), None);
        assert_eq!(target_for("FreeBSD amd64"), None);
        assert_eq!(target_for("Linux x86_64 extra"), None);
    }

    #[test]
    fn a_fresh_linux_machine_gets_the_binary_enrolls_with_the_ca_on_stdin_and_runs_as_a_user_service(
    ) {
        let r = Fake::new("Linux x86_64");
        let mut said = Vec::new();
        let f = install(&r, &machine(), &opts(), &mut |s| said.push(s)).unwrap();
        assert_eq!(f, Finish::Service("systemd"));
        assert_eq!(said, ["check", "binary", "enroll", "run"]);
        let log = r.log.borrow();
        let bin = log
            .iter()
            .find(|(c, _)| c.contains("agora.sha256"))
            .unwrap();
        assert!(bin.0.contains(
            "https://example.test/releases/download/v0.3.0/agora-x86_64-unknown-linux-gnu"
        ));
        assert!(bin.0.contains("ssh-keygen -Y verify"));
        assert_eq!(bin.1, "allowed-signers");
        let enroll = log.iter().find(|(c, _)| c.contains("node enroll")).unwrap();
        assert!(enroll
            .0
            .contains("--hub wss://localhost:17443 --ca - --code ABCD-2345 --id vps"));
        assert_eq!(enroll.1, "CA-PEM");
        // The code is on the command line of that one step and nowhere else.
        assert_eq!(
            log.iter()
                .filter(|(c, s)| c.contains("ABCD-2345") || s.contains("ABCD-2345"))
                .count(),
            1
        );
        assert!(log
            .last()
            .unwrap()
            .0
            .contains("systemctl --user enable --now"));
    }

    #[test]
    fn a_machine_that_already_has_this_version_skips_the_download() {
        let mut r = Fake::new("Darwin arm64");
        r.version = Some("0.3.0");
        let mut said = Vec::new();
        let f = install(&r, &machine(), &opts(), &mut |s| said.push(s)).unwrap();
        assert_eq!(f, Finish::Service("launchd"));
        assert_eq!(said, ["check", "enroll", "run"]);
        assert!(r.log.borrow().iter().all(|(c, _)| !c.contains("curl")));
    }

    #[test]
    fn without_a_pinned_key_it_stops_unless_asked_for_a_checksum_only_install() {
        let r = Fake::new("Linux x86_64");
        let mut o = opts();
        o.release_key = None;
        let e = install(&r, &machine(), &o, &mut |_| {}).unwrap_err();
        assert_eq!(
            (e.step, e.done.as_slice()),
            ("binary", ["check"].as_slice())
        );
        assert!(r.log.borrow().iter().all(|(c, _)| !c.contains("curl")));
        o.unsigned = true;
        let r = Fake::new("Linux x86_64");
        install(&r, &machine(), &o, &mut |_| {}).unwrap();
        let log = r.log.borrow();
        let bin = log
            .iter()
            .find(|(c, _)| c.contains("agora.sha256"))
            .unwrap();
        assert!(!bin.0.contains("ssh-keygen"));
        assert!(bin.0.contains("checksum mismatch"));
    }

    #[test]
    fn a_failing_step_stops_with_its_name_and_what_was_done_and_nothing_after_it_runs() {
        let mut r = Fake::new("Linux x86_64");
        r.fail_on = Some("node enroll");
        let e = install(&r, &machine(), &opts(), &mut |_| {}).unwrap_err();
        assert_eq!(e.step, "enroll");
        assert_eq!(e.done, ["check", "binary"]);
        assert_eq!(e.detail, "it broke");
        assert!(r
            .log
            .borrow()
            .iter()
            .all(|(c, _)| !c.contains("systemctl --user enable")));
    }

    #[test]
    fn an_unsupported_system_stops_at_the_check_and_no_systemd_means_a_tmux_hint() {
        let r = Fake::new("Linux riscv64");
        let e = install(&r, &machine(), &opts(), &mut |_| {}).unwrap_err();
        assert_eq!(e.step, "check");
        let mut r = Fake::new("Linux aarch64");
        r.systemd = false;
        let f = install(&r, &machine(), &opts(), &mut |_| {}).unwrap();
        assert!(matches!(f, Finish::Manual(c) if c.contains("tmux")));
    }

    #[test]
    fn values_that_could_become_shell_are_refused_before_anything_runs() {
        let r = Fake::new("Linux x86_64");
        let mut o = opts();
        o.code = "A; rm -rf ~".into();
        assert!(install(&r, &machine(), &o, &mut |_| {}).is_err());
        let mut o = opts();
        o.release_base = "https://x.test/'; id; '".into();
        assert!(install(&r, &machine(), &o, &mut |_| {}).is_err());
        assert!(
            r.log.borrow().is_empty() || r.log.borrow().iter().all(|(c, _)| !c.contains("id;"))
        );
    }

    #[test]
    fn a_failed_service_check_stops_the_install_instead_of_passing_for_success() {
        let mut r = Fake::new("Linux x86_64");
        r.uname_s_fails = true;
        let e = install(&r, &machine(), &opts(), &mut |_| {}).unwrap_err();
        assert_eq!((e.step, e.detail.as_str()), ("run", "connection reset"));
        assert_eq!(e.done, ["check", "binary", "enroll"]);
    }

    #[test]
    fn only_the_binary_the_install_would_run_counts_as_already_installed() {
        let r = Fake::new("Linux x86_64");
        install(&r, &machine(), &opts(), &mut |_| {}).unwrap();
        let log = r.log.borrow();
        let check = log.iter().find(|(c, _)| c.contains("--version")).unwrap();
        // A copy elsewhere on PATH is not used by the enroll and run steps, so it must not skip the install.
        assert!(check.0.contains("$HOME/.local/bin/agora") && !check.0.contains("|| agora"));
    }
}
