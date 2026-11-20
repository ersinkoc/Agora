//! `agora doctor`: checks the things that most often explain "it does not work" and reports each as a stable
//! id, a status and a short machine detail (docs/design/observability.md). No user-facing sentences here: the
//! CLI renders them from `locales/`. Nothing secret is collected: paths, versions and counts only.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    /// Works, but something worth knowing (no node running, an agent not installed).
    Warn,
    /// Agora cannot do its job on this machine until this is fixed.
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// Stable id, also the suffix of the locale key (`doctor-<id>`).
    pub id: &'static str,
    pub status: Status,
    /// Version, path, count or name, whatever the sentence needs as `$detail`.
    pub detail: String,
    /// More named values for the sentence, beside `$detail` (an agent kind's `$program` and `$from`).
    pub args: Vec<(&'static str, String)>,
}

fn check(id: &'static str, status: Status, detail: impl Into<String>) -> Check {
    Check {
        id,
        status,
        detail: detail.into(),
        args: Vec::new(),
    }
}

pub struct Inputs {
    pub data_dir: PathBuf,
    pub socket: PathBuf,
    pub manifests_dir: PathBuf,
}

/// Is `program` runnable: an existing path, or found in a `PATH` directory.
pub fn on_path(program: &str) -> bool {
    locate(program).is_some()
}

/// Where `program` is: an existing path, or the file a `PATH` directory holds under that name. Only looks at the file system:
/// nothing is run and nothing is read from the file.
pub fn locate(program: &str) -> Option<PathBuf> {
    let p = Path::new(program);
    if p.components().count() > 1 {
        return p.is_file().then(|| p.to_path_buf());
    }
    // A name the node starts by another file's full path (on Windows, a `.cmd` shim such as `npx`) is installed when that file is.
    let started_as = agora_platform::program_path(program, None);
    if started_as != p {
        return started_as.is_file().then_some(started_as);
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).find_map(|dir| {
        let plain = dir.join(program);
        if plain.is_file() {
            return Some(plain);
        }
        let exe = dir.join(format!("{program}.exe"));
        (cfg!(windows) && exe.is_file()).then_some(exe)
    })
}

/// A file the owner may not run (ADR-0087 rung 1: "broken"): found, but without the execute bit. Where the platform has no such bit
/// the file counts as runnable.
fn runnable(file: &Path) -> bool {
    cfg!(windows)
        || std::fs::metadata(file)
            .map(|m| agora_platform::is_executable(&m))
            .unwrap_or(false)
}

pub fn run(inputs: &Inputs) -> Vec<Check> {
    let mut out = vec![check("version", Status::Ok, env!("CARGO_PKG_VERSION"))];

    out.push(if agora_platform::PTY_SUPPORTED {
        check("pty", Status::Ok, "")
    } else {
        check("pty", Status::Fail, std::env::consts::OS)
    });
    out.push(if agora_platform::LOCAL_SOCKETS_SUPPORTED {
        check("local-sockets", Status::Ok, "")
    } else {
        check("local-sockets", Status::Warn, std::env::consts::OS)
    });

    // What the node's policy asks for, and whether this machine can do it (ADR-0047).
    let policy_file = inputs.data_dir.join(crate::daemon::POLICY_FILE);
    let policy = crate::daemon::read_policy(&policy_file);
    let sandbox = policy.sandbox.clone();
    let level = sandbox.level;
    // Whether the level the policy asks for can be enforced here (`worktree-offline` needs network rules too); with `off`, whether
    // confinement would be possible at all.
    let enforceable = if level == crate::sandbox::Level::Off {
        agora_platform::sandbox_supported()
    } else {
        sandbox.enforceable()
    };
    out.push(check(
        "sandbox",
        if enforceable {
            Status::Ok
        } else {
            Status::Warn
        },
        level.name(),
    ));
    if level != crate::sandbox::Level::Off {
        out.extend(policy_paths(&policy_file));
        // ADR-0083: what keeps a confined agent off the user's other sockets here, and which variables the node leaves out.
        out.extend(socket_checks(&sandbox));
        // ADR-0078: a confined agent commits with the identity the node copies from the person's git configuration.
        if crate::gitconfig::identity_missing() {
            out.push(check("git-identity-missing", Status::Warn, ""));
        }
        // ADR-0082: where the control socket needs its token, a folder agents may read or write must not hold the token file.
        if agora_platform::CONTROL_TOKEN_REQUIRED {
            let file = crate::control_token::token_path(&inputs.socket);
            let file = std::path::absolute(&file).unwrap_or(file);
            if file.parent().is_some_and(Path::is_dir) {
                let mut probe = sandbox.clone();
                probe.control_token = Some(file.clone());
                let profile =
                    probe.profile("doctor", None, Path::new("/nonexistent-agora-doctor"), "");
                if profile.is_some_and(|p| probe.token_reachable(&p)) {
                    out.push(check(
                        "control-token-reachable",
                        Status::Fail,
                        file.display().to_string(),
                    ));
                }
            }
        }
    }
    out.extend(authority(&inputs.data_dir, &policy, now_ms()));

    out.push(
        match agora_platform::no_console_window(&mut Command::new("git"))
            .arg("--version")
            .output()
        {
            Ok(o) if o.status.success() => check(
                "git",
                Status::Ok,
                String::from_utf8_lossy(&o.stdout).trim().to_string(),
            ),
            _ => check("git", Status::Fail, ""),
        },
    );

    out.push(data_dir(&inputs.data_dir));

    let running = agora_platform::LOCAL_SOCKETS_SUPPORTED
        && agora_platform::local_connect(&inputs.socket).is_ok();
    out.push(if running {
        check("node", Status::Ok, inputs.socket.display().to_string())
    } else {
        check("node", Status::Warn, inputs.socket.display().to_string())
    });

    out.extend(policy_read_at_start(&policy_file, &inputs.socket, running));
    out.extend(manifests(&inputs.manifests_dir));
    out.extend(credential_checks(
        &inputs.manifests_dir,
        &crate::credentials::SecretStore::in_data(&inputs.data_dir),
        &policy,
        level != crate::sandbox::Level::Off,
        |n| std::env::var(n).ok(),
    ));
    out
}

/// ADR-0084, per kind whose manifest declares `credential_env`. Names only, never values, and no claim about whether the agent is
/// logged in: the node only knows whether it holds something to give.
/// - `credential` (detail: the kind; `$names`): the policy lists names for it and a value exists for them (stored, or in this
///   process's environment).
/// - `credential-none` (detail: the kind; `$name`: the one to suggest, `$confined`): nothing would be passed. A warning where agents are
///   confined (an agent may not see its own login there), a note otherwise.
/// - `credential-unlisted` (detail: the kind; `$names`): a value exists for its declared names but no policy line lets it receive them.
/// - `credential-refused` (detail: the kind; `$names`): the policy lists names the manifest does not declare; they are not passed.
/// - `credential-malformed` (detail: the kind): the `credentials.<kind>` line is malformed and passes nothing.
fn credential_checks(
    manifests_dir: &Path,
    store: &crate::credentials::SecretStore,
    policy: &crate::daemon::Policy,
    confined: bool,
    env: impl Fn(&str) -> Option<String>,
) -> Vec<Check> {
    let creds = crate::credentials::Credentials::new(store.clone(), policy.credentials.clone());
    let (loaded, _) = crate::manifests::load(manifests_dir);
    let mut out = Vec::new();
    // The same lookup the launch path makes: a stored file counts only when it reads back as a well-formed value.
    let has = |n: &str| {
        store.get(n).is_some()
            || env(n).is_some_and(|v| crate::credentials::Secret::new(&v).is_ok())
    };
    for l in loaded {
        let Some(own) = l
            .manifest
            .accounts
            .as_ref()
            .map(|a| a.credential_env.clone())
            .filter(|n| !n.is_empty())
        else {
            continue;
        };
        let kind = l.manifest.kind.clone();
        let mut push = |id: &'static str, status: Status, names: String, name: &str| {
            let mut c = check(id, status, kind.clone());
            c.args = vec![
                ("names", names),
                ("name", name.to_string()),
                ("confined", confined.to_string()),
            ];
            out.push(c);
        };
        if policy.credentials_malformed.contains(&kind) {
            push("credential-malformed", Status::Warn, String::new(), &own[0]);
        }
        let refused = creds.undeclared(&kind, &own);
        if !refused.is_empty() {
            push(
                "credential-refused",
                Status::Warn,
                refused.join(", "),
                &own[0],
            );
        }
        let given: Vec<String> = creds
            .listed(&kind)
            .iter()
            .filter(|n| own.contains(n) && has(n))
            .cloned()
            .collect();
        if !given.is_empty() {
            push("credential", Status::Ok, given.join(", "), &own[0]);
            continue;
        }
        let unlisted: Vec<String> = own.iter().filter(|n| has(n)).cloned().collect();
        if !unlisted.is_empty() && creds.listed(&kind).is_empty() {
            push(
                "credential-unlisted",
                Status::Warn,
                unlisted.join(", "),
                &own[0],
            );
        } else {
            push(
                "credential-none",
                if confined { Status::Warn } else { Status::Ok },
                String::new(),
                &own[0],
            );
        }
    }
    out
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Who may make this machine act, as the node applies its policy file and trusted-devices file when it starts (the architecture
/// review's R1: setup and diagnostics state the effective rules, so nobody assumes a protection that is not configured).
/// - `signed-commands` (detail: the gated commands), `signed-commands-team` (no `signed_commands` line, but a device limited to
///   projects turns on the team default) or `signed-commands-none` (a note; detail: the policy file): whether commands routed through
///   the hub need a device-signed grant (ADR-0041). Without one, whoever controls the hub can send them (threat model T1); starting
///   an agent in a folder from the window is signed regardless (ADR-0052).
/// - `devices` (detail: how many devices are trusted now; `$unlimited`: how many of them may act in every project) or `devices-none`.
/// - `skip-permissions` (detail: `allow`, `forbid` or `confined_only`; ADR-0059).
/// - `policy-problems` (detail: how many lines the node did not understand; any of them forbids skipping permissions).
fn authority(data_dir: &Path, policy: &crate::daemon::Policy, now_ms: u64) -> Vec<Check> {
    let mut out = Vec::new();
    let signed = policy.effective_signed_commands();
    out.push(if signed.0.is_empty() {
        check(
            "signed-commands-none",
            Status::Warn,
            data_dir
                .join(crate::daemon::POLICY_FILE)
                .display()
                .to_string(),
        )
    } else if policy.signed_commands.0.is_empty() {
        check("signed-commands-team", Status::Ok, signed.0.join(", "))
    } else {
        check("signed-commands", Status::Ok, signed.0.join(", "))
    });

    let (entries, _) =
        crate::device_trust::read_entries(&data_dir.join(crate::approvals::TRUSTED_DEVICES_FILE));
    let live: Vec<&crate::device_trust::Entry> = entries
        .iter()
        .filter(|e| crate::device_trust::alive(&entries, e, now_ms))
        .collect();
    out.push(if live.is_empty() {
        check("devices-none", Status::Warn, "")
    } else {
        let unlimited = live
            .iter()
            .filter(|e| {
                crate::device_trust::effective_limit(&entries, &policy.device_projects, &e.id)
                    .is_none()
            })
            .count();
        let mut c = check("devices", Status::Ok, live.len().to_string());
        c.args = vec![("unlimited", unlimited.to_string())];
        c
    });

    out.push(check(
        "skip-permissions",
        Status::Ok,
        policy.skip_permissions.name(),
    ));
    if policy.show_secret {
        out.push(check("show-secret", Status::Warn, "true"));
    }
    if !policy.problems.is_empty() {
        out.push(check(
            "policy-problems",
            Status::Warn,
            policy.problems.len().to_string(),
        ));
    }
    out
}

/// ADR-0083: whether this machine refuses a confined agent's connections to unix sockets it was not given (`sockets-profile` on macOS: measured on a real Mac for the agent's own socket, another one and the resolver; or why
/// not: `sockets-abstract`, `sockets-none`), which socket variables the node leaves out of an agent's environment
/// (`env-scrubbed`) and which of them the owner let through (`env-passed`). Names only, never values.
fn socket_checks(sandbox: &crate::sandbox::SandboxPolicy) -> Vec<Check> {
    use agora_platform::SocketConfinement as C;
    let mut out = vec![match agora_platform::socket_confinement() {
        // Measured on a real Mac (2026-11-17): own socket open, another closed, resolver open. Other system sockets are not (the text says so).
        C::Full => check("sockets-profile", Status::Ok, ""),
        C::AbstractOnly => check("sockets-abstract", Status::Warn, ""),
        C::None => check("sockets-none", Status::Warn, std::env::consts::OS),
    }];
    out.push(check(
        "env-scrubbed",
        Status::Ok,
        sandbox.scrubbed_env().join(", "),
    ));
    if !sandbox.pass_env.is_empty() {
        out.push(check(
            "env-passed",
            Status::Warn,
            sandbox.pass_env.join(", "),
        ));
    }
    out
}

/// The node reads its policy file (`<data>/policy`: limited devices, signed commands, the sandbox) only when it starts, unlike the
/// trusted-devices file, which it follows. With a policy file: a note that a change needs a restart, and a warning when the file
/// changed after the running node started (its socket was made then; where that cannot be told, only the note). Detail: the path.
fn policy_read_at_start(policy: &Path, socket: &Path, running: bool) -> Option<Check> {
    let changed = std::fs::metadata(policy).ok()?.modified().ok()?;
    let detail = policy.display().to_string();
    let started = running
        .then(|| std::fs::metadata(socket).ok()?.modified().ok())
        .flatten();
    Some(match started {
        Some(at) if changed > at => check("policy-changed", Status::Warn, detail),
        _ => check("policy-restart", Status::Ok, detail),
    })
}

/// One note per `sandbox.read` / `sandbox.write` entry the node cannot use: dropped (`policy-path-dropped`: relative, `..`, `~` with
/// no home folder) or not there (`policy-path-missing`: skipped until it exists). Detail: `<key> = <entry as written>`. Notes, not
/// problems: agents still start, without that path.
fn policy_paths(file: &Path) -> Vec<Check> {
    crate::daemon::policy_sandbox_entries(file)
        .into_iter()
        .filter_map(|(key, raw)| {
            let detail = format!("{key} = {raw}");
            match crate::sandbox::expand(&raw) {
                None => Some(check("policy-path-dropped", Status::Warn, detail)),
                Some(p) if !p.exists() => Some(check("policy-path-missing", Status::Warn, detail)),
                Some(_) => None,
            }
        })
        .collect()
}

fn data_dir(dir: &Path) -> Check {
    let probe = dir.join(format!(".doctor-{}", std::process::id()));
    // `create_new` never follows a link planted at the probe's name.
    let ok = std::fs::create_dir_all(dir)
        .and_then(|()| {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&probe)?
                .write_all(b"x")
        })
        .is_ok();
    let _ = std::fs::remove_file(&probe);
    check(
        "data-dir",
        if ok { Status::Ok } else { Status::Fail },
        dir.display().to_string(),
    )
}

/// Every agent kind the node offers (the built-in ones and the folder's, `crate::manifests`): where it came from (`$from`: `built-in`
/// or `folder`), the program looked for (`$program`) and whether it is found (`agent-installed`), not found (`agent-missing`) or found
/// without the right to run it (`agent-broken`). The agent's own version is not shown: no manifest has a version command yet. A folder file that cannot be read is named; its
/// built-in kind, if any, stays. A missing folder is fine: the built-in kinds still load.
fn manifests(dir: &Path) -> Vec<Check> {
    let (loaded, problems) = crate::manifests::load(dir);
    let mut out = vec![check("manifests", Status::Ok, loaded.len().to_string())];
    for l in loaded {
        let program = crate::manifests::program(&l.manifest)
            .unwrap_or_default()
            .to_string();
        let found = if program.is_empty() {
            None
        } else {
            locate(&program)
        };
        // Found but not runnable is "broken": the program is there and starting it would fail (ADR-0087 rung 1). Nothing is run to
        // find out, so a program that starts and then fails its own checks is not caught here.
        let broken = found.as_deref().is_some_and(|f| !runnable(f));
        let installed = found.is_some() && !broken;
        let mut c = check(
            if installed {
                "agent-installed"
            } else if broken {
                "agent-broken"
            } else {
                "agent-missing"
            },
            if installed { Status::Ok } else { Status::Warn },
            l.manifest.kind,
        );
        c.args = vec![("program", program), ("from", l.source.name().to_string())];
        out.push(c);
    }
    for (p, _) in problems {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        out.push(check("manifest-invalid", Status::Warn, name));
    }
    out.extend(older_than_built_in(dir));
    out
}

/// A manifest in the folder replaces the built-in one of its kind, so a copy made for an older release keeps replacing it after the
/// program is updated: it lacks what the newer built-in has (a session id to resume with, a skip-permissions option, the prompt
/// patterns). Found on a real machine: such copies from 11 Nov made `claude-code` agents impossible to resume. One warning for each
/// folder file whose version is older than the built-in one of the same kind, naming the file (never its path).
fn older_than_built_in(dir: &Path) -> Vec<Check> {
    let (loaded, _) = crate::manifests::load(dir);
    let mut out = Vec::new();
    for l in loaded {
        let crate::manifests::Source::Folder(path) = &l.source else {
            continue;
        };
        let Some(shipped) = crate::manifests::BUILT_IN
            .iter()
            .filter_map(|(_, text)| agora_adapters::manifest::Manifest::parse(text).ok())
            .find(|m| m.kind == l.manifest.kind)
        else {
            continue;
        };
        if !version_older(&l.manifest.version, &shipped.version) {
            continue;
        }
        let mut c = check("manifest-older", Status::Warn, l.manifest.kind.clone());
        c.args = vec![
            ("from", l.manifest.version.clone()),
            ("shipped", shipped.version),
            (
                "file",
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
        ];
        out.push(c);
    }
    out
}

/// Whether manifest version `a` is older than `b`: both are dotted numbers (`2026.11.13.1`), compared part by part. Anything else is
/// not compared (no warning for a version in another shape).
fn version_older(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Option<Vec<u64>> { v.split('.').map(|p| p.parse().ok()).collect() };
    match (parts(a), parts(b)) {
        (Some(a), Some(b)) => a < b,
        _ => false,
    }
}

/// ADR-0093: a notice when the holder running here speaks only link version 1 (built before 0.3.27), so structured agents are resumed
/// after a node restart, not kept. Read only: it looks at the note the node leaves (`holder_note`), never at the holder or an agent.
/// A warning, never a failure: it does not change the exit status. `detail` is how many agents the holder keeps.
pub fn holder_checks(home: &Path) -> Vec<Check> {
    match crate::holder_note::old_holder(home) {
        Some(h) => vec![check("holder-old", Status::Warn, h.kept.to_string())],
        None => Vec::new(),
    }
}

/// True when nothing failed (warnings are fine): the exit status of `agora doctor`.
pub fn healthy(checks: &[Check]) -> bool {
    checks.iter().all(|c| c.status != Status::Fail)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agora-doctor-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn find<'a>(c: &'a [Check], id: &str) -> Vec<&'a Check> {
        c.iter().filter(|c| c.id == id).collect()
    }

    const MANIFEST: &str = "kind = \"demo\"\ndisplay_name = \"Demo\"\nversion = \"1\"\nmin_engine = 1\n[process]\nnames = [\"demo\"]\nlaunch = [\"LAUNCH\"]\n";

    #[test]
    fn healthy_machine_with_no_node_running() {
        let d = tmp("healthy");
        let checks = run(&Inputs {
            data_dir: d.join("data"),
            socket: d.join("nope.sock"),
            manifests_dir: d.join("manifests"),
        });
        assert_eq!(find(&checks, "version")[0].status, Status::Ok);
        assert_eq!(find(&checks, "data-dir")[0].status, Status::Ok);
        assert_eq!(find(&checks, "node")[0].status, Status::Warn);
        // No manifests folder: the built-in kinds still load.
        assert_eq!(find(&checks, "manifests")[0].status, Status::Ok);
        assert_eq!(
            find(&checks, "manifests")[0].detail,
            (crate::manifests::BUILT_IN.len() + 1).to_string()
        );
        // Where terminals are unsupported (Windows for now) that is a reported problem, not a healthy machine.
        let git_ok = find(&checks, "git")[0].status == Status::Ok;
        assert_eq!(healthy(&checks), git_ok && agora_platform::PTY_SUPPORTED);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_policy_file_is_said_to_need_a_restart_and_a_change_after_the_start_is_a_warning() {
        let d = tmp("policy-restart");
        let policy = d.join("policy");
        let socket = d.join("node.sock");
        let at = |p: &Path, t: std::time::SystemTime| {
            std::fs::File::options()
                .write(true)
                .open(p)
                .unwrap()
                .set_modified(t)
                .unwrap();
        };
        let now = std::time::SystemTime::now();
        let minute = std::time::Duration::from_secs(60);
        // No policy file: nothing to say.
        assert!(policy_read_at_start(&policy, &socket, true).is_none());
        std::fs::write(&policy, "device.web-1.projects = shop\n").unwrap();
        std::fs::write(&socket, "").unwrap();
        // No node running, or one started after the last change: the note.
        for running in [false, true] {
            at(&socket, now);
            at(&policy, now - minute);
            let c = policy_read_at_start(&policy, &socket, running).unwrap();
            assert_eq!((c.id, c.status), ("policy-restart", Status::Ok));
            assert_eq!(c.detail, policy.display().to_string());
        }
        // Changed after the running node started: the warning, which is a note and not a problem.
        at(&socket, now - minute);
        at(&policy, now);
        let c = policy_read_at_start(&policy, &socket, true).unwrap();
        assert_eq!((c.id, c.status), ("policy-changed", Status::Warn));
        assert!(healthy(&[c]));
        // Where the start cannot be told (no socket file, e.g. a named pipe), only the note.
        std::fs::remove_file(&socket).unwrap();
        assert_eq!(
            policy_read_at_start(&policy, &socket, true).unwrap().id,
            "policy-restart"
        );
        // `run` says it too.
        let checks = run(&Inputs {
            data_dir: d.clone(),
            socket: d.join("nope.sock"),
            manifests_dir: d.join("manifests"),
        });
        assert_eq!(find(&checks, "policy-restart").len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn doctor_says_what_restricts_unix_sockets_and_which_variables_are_scrubbed_or_passed() {
        let dir = tmp("sockets");
        let data = dir.join("data");
        std::fs::create_dir_all(&data).unwrap();
        let inputs = Inputs {
            data_dir: data.clone(),
            socket: dir.join("none.sock"),
            manifests_dir: dir.join("manifests"),
        };
        let ids = |checks: &[Check]| -> Vec<(&'static str, String)> {
            checks
                .iter()
                .filter(|c| c.id.starts_with("sockets-") || c.id.starts_with("env-"))
                .map(|c| (c.id, c.detail.clone()))
                .collect()
        };
        // Not confined: nothing is said about what confinement does.
        std::fs::write(data.join(crate::daemon::POLICY_FILE), "sandbox = off\n").unwrap();
        assert!(ids(&run(&inputs)).is_empty());
        std::fs::write(
            data.join(crate::daemon::POLICY_FILE),
            "sandbox = worktree\nsandbox.pass_env = DOCKER_HOST\n",
        )
        .unwrap();
        let got = ids(&run(&inputs));
        let restriction = match agora_platform::socket_confinement() {
            agora_platform::SocketConfinement::Full => "sockets-profile",
            agora_platform::SocketConfinement::AbstractOnly => "sockets-abstract",
            agora_platform::SocketConfinement::None => "sockets-none",
        };
        assert_eq!(got[0].0, restriction, "{got:?}");
        let scrubbed = &got.iter().find(|c| c.0 == "env-scrubbed").unwrap().1;
        assert!(scrubbed.contains("SSH_AUTH_SOCK") && !scrubbed.contains("DOCKER_HOST"));
        assert_eq!(
            got.iter()
                .find(|c| c.0 == "env-passed")
                .map(|c| c.1.as_str()),
            Some("DOCKER_HOST")
        );
    }

    #[test]
    fn sandbox_policy_paths_that_cannot_be_used_are_named_with_their_line() {
        let dir = tmp("policy-paths");
        let data = dir.join("data");
        std::fs::create_dir_all(&data).unwrap();
        let here = dir.join("here");
        std::fs::create_dir_all(&here).unwrap();
        let gone = dir.join("not-there");
        std::fs::write(
            data.join(crate::daemon::POLICY_FILE),
            format!(
                "sandbox = worktree\nsandbox.read = {}, relative/dir\nsandbox.write = {}, ../up\n",
                here.display(),
                gone.display()
            ),
        )
        .unwrap();
        let inputs = Inputs {
            data_dir: data.clone(),
            socket: dir.join("none.sock"),
            manifests_dir: dir.join("manifests"),
        };
        let checks = run(&inputs);
        let found = |id: &str| -> Vec<String> {
            checks
                .iter()
                .filter(|c| c.id == id)
                .map(|c| c.detail.clone())
                .collect()
        };
        assert_eq!(
            found("policy-path-dropped"),
            ["sandbox.read = relative/dir", "sandbox.write = ../up"]
        );
        assert_eq!(
            found("policy-path-missing"),
            [format!("sandbox.write = {}", gone.display())]
        );
        assert!(checks
            .iter()
            .filter(|c| c.id.starts_with("policy-path"))
            .all(|c| c.status == Status::Warn));
        // With confinement off the paths do not matter and nothing is said.
        std::fs::write(
            data.join(crate::daemon::POLICY_FILE),
            "sandbox = off\nsandbox.read = relative/dir\n",
        )
        .unwrap();
        assert!(!run(&inputs).iter().any(|c| c.id.starts_with("policy-path")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_folder_manifest_older_than_the_built_in_one_is_warned_about() {
        let dir = tmp("older-manifests");
        let shipped = crate::manifests::BUILT_IN
            .iter()
            .find(|(n, _)| *n == "claude-code.toml")
            .unwrap()
            .1;
        let version = shipped
            .lines()
            .find(|l| l.starts_with("version = "))
            .unwrap()
            .to_string();
        let with = |v: &str| shipped.replace(&version, &format!("version = \"{v}\""));
        // Older: warned, and the file (not its path) is named.
        std::fs::write(dir.join("claude-code.toml"), with("2026.01.01.1")).unwrap();
        // The same version, a newer one and a kind with no built-in one are fine.
        std::fs::write(dir.join("codex.toml"), {
            let c = crate::manifests::BUILT_IN
                .iter()
                .find(|(n, _)| *n == "codex.toml")
                .unwrap()
                .1;
            c.to_string()
        })
        .unwrap();
        std::fs::write(
            dir.join("mine.toml"),
            "kind = \"mine\"\ndisplay_name = \"M\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"mine\"]\n",
        )
        .unwrap();
        let checks = manifests(&dir);
        let older = find(&checks, "manifest-older");
        assert_eq!(older.len(), 1, "{checks:?}");
        assert_eq!(older[0].detail, "claude-code");
        assert!(older[0]
            .args
            .iter()
            .any(|(k, v)| *k == "file" && v == "claude-code.toml"));
        assert!(older[0]
            .args
            .iter()
            .any(|(k, v)| *k == "from" && v == "2026.01.01.1"));
        // A newer folder copy is not warned about.
        std::fs::write(dir.join("claude-code.toml"), with("2099.01.01.1")).unwrap();
        assert!(find(&manifests(&dir), "manifest-older").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_versions_compare_part_by_part_and_other_shapes_are_not_compared() {
        assert!(version_older("2026.11.09.1", "2026.11.13.1"));
        assert!(version_older("2026.11.13.1", "2026.11.13.2"));
        assert!(version_older("2026.9.30.1", "2026.11.1.1")); // numbers, not text
        assert!(!version_older("2026.11.13.1", "2026.11.13.1"));
        assert!(!version_older("2027.1.1.1", "2026.11.13.1"));
        assert!(version_older("1", "2026.11.13.1")); // a hand-made override with a plain number counts as older
        assert!(!version_older("v2", "2026.11.13.1"));
        assert!(!version_older("", "2026.11.13.1"));
    }

    #[test]
    fn manifests_report_installed_missing_and_invalid() {
        let d = tmp("manifests");
        let m = d.join("m");
        std::fs::create_dir_all(&m).unwrap();
        // `sh` exists on Unix CI; the test only needs some program that is certainly on PATH.
        let present = if cfg!(windows) { "cmd" } else { "sh" };
        std::fs::write(m.join("a.toml"), MANIFEST.replace("LAUNCH", present)).unwrap();
        std::fs::write(
            m.join("b.toml"),
            MANIFEST
                .replace("demo", "ghost")
                .replace("LAUNCH", "definitely-not-installed-xyz"),
        )
        .unwrap();
        std::fs::write(m.join("c.toml"), "not = [valid").unwrap();
        let checks = manifests(&m);
        assert_eq!(checks[0].id, "manifests");
        assert_eq!(
            checks[0].detail,
            (crate::manifests::BUILT_IN.len() + 3).to_string()
        );
        let of = |kind: &str| checks.iter().find(|c| c.detail == kind).unwrap();
        assert_eq!(of("demo").id, "agent-installed");
        assert_eq!(
            of("demo").args,
            [("program", present.to_string()), ("from", "folder".into())]
        );
        assert_eq!(of("ghost").id, "agent-missing");
        assert_eq!(of("ghost").args[0].1, "definitely-not-installed-xyz");
        assert_eq!(of("codex").args[1].1, "built-in");
        assert_eq!(of("codex").args[0].1, "codex");
        assert_eq!(find(&checks, "manifest-invalid")[0].detail, "c.toml");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_program_without_the_right_to_run_is_broken_not_missing() {
        if cfg!(windows) {
            return; // no execute bit there
        }
        let d = tmp("broken");
        let m = d.join("m");
        std::fs::create_dir_all(&m).unwrap();
        let prog = d.join("agent-bin");
        std::fs::write(&prog, "#!/bin/sh\n").unwrap();
        agora_platform::set_executable(&prog, false).unwrap();
        let text = MANIFEST.replace("LAUNCH", &prog.display().to_string());
        std::fs::write(m.join("a.toml"), &text).unwrap();
        let of = |checks: &[Check]| checks.iter().find(|c| c.detail == "demo").unwrap().id;
        assert_eq!(of(&manifests(&m)), "agent-broken");
        agora_platform::set_executable(&prog, true).unwrap();
        assert_eq!(of(&manifests(&m)), "agent-installed");
        std::fs::remove_file(&prog).unwrap();
        assert_eq!(of(&manifests(&m)), "agent-missing");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn unwritable_data_dir_is_a_failure() {
        let d = tmp("ro");
        // A file where the directory should be cannot be created or written into.
        let blocker = d.join("file");
        std::fs::write(&blocker, b"x").unwrap();
        let c = data_dir(&blocker.join("sub"));
        assert_eq!(c.status, Status::Fail);
        assert!(!healthy(&[c]));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn on_path_finds_real_programs_only() {
        assert!(on_path(if cfg!(windows) { "cmd" } else { "sh" }));
        assert!(!on_path("definitely-not-installed-xyz"));
    }

    fn authority_of(name: &str, policy: &str, trusted: &str) -> Vec<Check> {
        let d = tmp(name);
        if !policy.is_empty() {
            std::fs::write(d.join(crate::daemon::POLICY_FILE), policy).unwrap();
        }
        if !trusted.is_empty() {
            std::fs::write(d.join(crate::approvals::TRUSTED_DEVICES_FILE), trusted).unwrap();
        }
        let p = crate::daemon::read_policy(&d.join(crate::daemon::POLICY_FILE));
        let out = authority(&d, &p, 1_000);
        let _ = std::fs::remove_dir_all(&d);
        out
    }

    fn key(n: u8) -> String {
        format!("04{}", format!("{n:02x}").repeat(64))
    }

    #[test]
    fn showing_secret_files_is_warned_about_only_when_on() {
        assert!(find(&authority_of("secret-default", "", ""), "show-secret").is_empty());
        for off in [
            "files.show_secret = false\n",
            "files.show_secret = yes\n",
            "files.show_secret = TRUE\n",
        ] {
            let c = authority_of("secret-off", off, "");
            assert!(find(&c, "show-secret").is_empty(), "{off}");
        }
        let c = authority_of("secret-on", "files.show_secret = true\n", "");
        assert_eq!(find(&c, "show-secret")[0].status, Status::Warn);
    }

    #[test]
    fn without_a_policy_the_routed_commands_are_said_to_need_no_signature() {
        let c = authority_of("authority-default", "", "");
        let none = find(&c, "signed-commands-none");
        assert_eq!(none.len(), 1, "{c:?}");
        assert_eq!(none[0].status, Status::Warn);
        assert!(none[0].detail.ends_with("policy"), "{}", none[0].detail);
        assert_eq!(find(&c, "devices-none")[0].status, Status::Warn);
        assert_eq!(find(&c, "skip-permissions")[0].detail, "allow");
        assert!(find(&c, "policy-problems").is_empty());
    }

    #[test]
    fn a_signed_commands_line_is_listed_as_the_node_applies_it() {
        let c = authority_of(
            "authority-listed",
            "signed_commands = agent.start, terminal.*\nskip_permissions = confined_only\n",
            &format!("laptop {}\n", key(1)),
        );
        let s = find(&c, "signed-commands");
        assert_eq!(s[0].status, Status::Ok);
        assert_eq!(s[0].detail, "agent.start, terminal.*");
        let d = find(&c, "devices");
        assert_eq!(
            (d[0].detail.as_str(), d[0].args.clone()),
            ("1", vec![("unlimited", "1".to_string())])
        );
        assert_eq!(find(&c, "skip-permissions")[0].detail, "confined_only");
    }

    #[test]
    fn a_limited_device_turns_on_the_team_default_and_is_not_counted_as_unlimited() {
        let c = authority_of(
            "authority-team",
            "device.phone.projects = web\n",
            &format!("laptop {}\nphone {}\n", key(1), key(2)),
        );
        let t = find(&c, "signed-commands-team");
        assert_eq!(t.len(), 1, "{c:?}");
        assert_eq!(
            t[0].detail,
            crate::grants::SignedCommands::team_default().0.join(", ")
        );
        let d = find(&c, "devices");
        assert_eq!(d[0].detail, "2");
        assert_eq!(d[0].args, vec![("unlimited", "1".to_string())]);
    }

    #[test]
    fn an_expired_device_is_not_counted_and_a_misread_policy_is_a_note() {
        let c = authority_of(
            "authority-expired",
            "skip_permission = forbid\n",
            &format!(
                "laptop {} expires=500\nphone {} expires=5000\n",
                key(1),
                key(2)
            ),
        );
        // At 1 000 ms the laptop's trust has ended and the phone's has not.
        assert_eq!(find(&c, "devices")[0].detail, "1", "{c:?}");
        let p = find(&c, "policy-problems");
        assert_eq!((p[0].status, p[0].detail.as_str()), (Status::Warn, "1"));
        // An unreadable line forbids skipping permissions; the doctor says what the node does.
        assert_eq!(find(&c, "skip-permissions")[0].detail, "forbid");
    }
    #[test]
    fn credentials_are_reported_by_name_for_the_kinds_that_declare_them() {
        let d = tmp("credentials");
        let store = crate::credentials::SecretStore::in_data(&d);
        let no_env = |_: &str| None;
        let policy = |text: &str| {
            std::fs::write(d.join("policy"), text).unwrap();
            crate::daemon::read_policy(&d.join("policy"))
        };
        let kind =
            |c: &[Check], id: &str, kind: &str| c.iter().any(|c| c.id == id && c.detail == kind);
        // Nothing stored, nothing in the environment: a warning where confined, a note otherwise; only declared kinds appear.
        let c = credential_checks(&d.join("m"), &store, &policy(""), true, no_env);
        assert!(kind(&c, "credential-none", "claude-code") && kind(&c, "credential-none", "codex"));
        assert!(!c.iter().any(|c| c.detail == "wrongstack"));
        assert!(c.iter().all(|c| c.status == Status::Warn));
        let c = credential_checks(&d.join("m"), &store, &policy(""), false, no_env);
        assert!(c.iter().all(|c| c.status == Status::Ok));
        // The suggestion is the first declared name.
        let first = find(&c, "credential-none");
        let cc = first.iter().find(|c| c.detail == "claude-code").unwrap();
        assert!(cc
            .args
            .contains(&("name", "CLAUDE_CODE_OAUTH_TOKEN".to_string())));
        // A stored file that does not read back as a value is no credential: the node would pass nothing.
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(store.dir().join("CLAUDE_CODE_OAUTH_TOKEN"), "two\nlines").unwrap();
        let c = credential_checks(
            &d.join("m"),
            &store,
            &policy("credentials.claude-code = CLAUDE_CODE_OAUTH_TOKEN\n"),
            true,
            no_env,
        );
        assert!(kind(&c, "credential-none", "claude-code"), "{c:?}");
        assert!(!kind(&c, "credential", "claude-code"));
        // A stored value with no policy line is named, not passed; with the line it is given. Names only, in every field.
        store
            .set(
                "CLAUDE_CODE_OAUTH_TOKEN",
                &crate::credentials::Secret::new("sk-hidden-123").unwrap(),
            )
            .unwrap();
        let c = credential_checks(&d.join("m"), &store, &policy(""), true, no_env);
        assert!(kind(&c, "credential-unlisted", "claude-code"));
        let c = credential_checks(
            &d.join("m"),
            &store,
            &policy("credentials.claude-code = CLAUDE_CODE_OAUTH_TOKEN\ncredentials.codex = NOT_DECLARED\n"),
            true,
            no_env,
        );
        assert!(kind(&c, "credential", "claude-code"));
        assert!(kind(&c, "credential-refused", "codex"));
        let shown = format!("{c:?}");
        assert!(!shown.contains("sk-hidden-123"), "{shown}");
        // The node's own environment counts as a source.
        let c = credential_checks(
            &d.join("m"),
            &store,
            &policy("credentials.codex = OPENAI_API_KEY\n"),
            true,
            |n| (n == "OPENAI_API_KEY").then(|| "x".to_string()),
        );
        assert!(kind(&c, "credential", "codex"));
        // A malformed line is named and passes nothing.
        let c = credential_checks(
            &d.join("m"),
            &store,
            &policy("credentials.codex = A=1\n"),
            true,
            no_env,
        );
        assert!(kind(&c, "credential-malformed", "codex"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
