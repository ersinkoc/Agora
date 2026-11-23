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
    /// `<AGORA_HOME>`: where `agora agent install` keeps the programs it installed.
    pub agora_home: PathBuf,
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
    search(&std::env::var_os("PATH")?, program)
}

/// F66: as the launch searches the PATH, a file without the right to run is passed over for a later one that has it; only when none
/// can run is the first file named (the doctor then calls it broken).
fn search(paths: &std::ffi::OsStr, program: &str) -> Option<PathBuf> {
    let mut first = None;
    for dir in std::env::split_paths(paths) {
        let plain = dir.join(program);
        let exe = dir.join(format!("{program}.exe"));
        let file = if plain.is_file() {
            plain
        } else if cfg!(windows) && exe.is_file() {
            exe
        } else {
            continue;
        };
        if runnable(&file) {
            return Some(file);
        }
        first.get_or_insert(file);
    }
    first
}

/// Where an agent's program was found (ADR-0087 item 3 and 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// `agora agent install` put it under `<AGORA_HOME>/agents/bin`: the agent's folder name and its version.
    Agora(PathBuf, String),
    /// The user's own: a path, or a `PATH` directory.
    You(PathBuf),
}

impl Found {
    pub fn path(&self) -> &Path {
        match self {
            Found::Agora(p, _) | Found::You(p) => p,
        }
    }
}

/// Like [`locate`], but what `agora agent install` made under `<home>/agents/bin` comes first (only for a plain file name that the
/// installer's own records and checks accept, [`agora_adapters::agent_install::installed_program`]), then the user's `PATH`.
pub fn locate_in(home: &Path, program: &str) -> Option<Found> {
    if let Some(i) = agora_adapters::agent_install::installed_program(home, program) {
        return Some(Found::Agora(i.path, i.version));
    }
    locate(program).map(Found::You)
}

/// The program to start for `program`: the installed one's full path, else `program` unchanged.
pub fn start_program(home: &Path, program: &str) -> String {
    use agora_adapters::agent_install::{lookup_installed, Lookup};
    match lookup_installed(home, program) {
        Lookup::Installed(i) => i.path.to_str().map(str::to_string),
        Lookup::Unsafe => {
            // No secret here: only the program's file name.
            eprintln!("agent program {program:?} was installed by agora but its folder or file is not safe; using the PATH");
            None
        }
        Lookup::NotInstalled => None,
    }
    .unwrap_or_else(|| program.to_string())
}

/// An agent folder owns `program` but the installed copy failed its safety checks, so it is not used.
pub fn install_unsafe(home: &Path, program: &str) -> bool {
    agora_adapters::agent_install::lookup_installed(home, program)
        == agora_adapters::agent_install::Lookup::Unsafe
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
        // F68: each `git config` read is time-limited like the version probe below, so a git that hangs cannot hold the doctor.
        if identity_missing_within(crate::gitconfig::key_command, GIT_TIMEOUT) {
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

    out.push(match version_of("git", GIT_TIMEOUT) {
        Some(v) => check("git", Status::Ok, v),
        None => check("git", Status::Fail, ""),
    });

    out.push(data_dir(&inputs.data_dir));

    let running = agora_platform::LOCAL_SOCKETS_SUPPORTED
        && agora_platform::local_connect(&inputs.socket).is_ok();
    out.push(if running {
        check("node", Status::Ok, inputs.socket.display().to_string())
    } else {
        check("node", Status::Warn, inputs.socket.display().to_string())
    });

    out.extend(policy_read_at_start(&policy_file, &inputs.socket, running));
    out.extend(manifests(
        &inputs.manifests_dir,
        &inputs.agora_home,
        &policy.registry,
    ));
    out.extend(credential_checks(
        &inputs.manifests_dir,
        &crate::credentials::SecretStore::in_data(&inputs.data_dir),
        &policy,
        level != crate::sandbox::Level::Off,
        |n| std::env::var(n).ok(),
    ));
    out
}

/// How long each git command the doctor runs (`git --version`, the identity's `git config` reads) may take before it counts as not
/// working (F68).
const GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// The most of `git --version`'s output kept (the rest is read and dropped): a version is one short line.
const VERSION_MAX: usize = 1024;
/// The most of a `git config` read kept: more than a value the node copies (`gitconfig::MAX_VALUE`), so a long one is still seen.
const KEY_MAX: usize = 4096;

/// `gitconfig::identity_missing` with each key's `git config` (made by `command`) run by [`output_within`]: one that does not end
/// in time reads as not set (F68).
fn identity_missing_within(
    command: impl Fn(&str) -> Option<Command>,
    timeout: std::time::Duration,
) -> bool {
    crate::gitconfig::identity_missing_with(|key| {
        let (status, out) = output_within(command(key)?, timeout, KEY_MAX)?;
        crate::gitconfig::key_value(status, out)
    })
}

/// `<program> --version`, trimmed, when it ends successfully within `timeout` (at most `VERSION_MAX` bytes of it). F68: one that does
/// not end counts as not working ([`output_within`]).
fn version_of(program: &str, timeout: std::time::Duration) -> Option<String> {
    let mut cmd = Command::new(program);
    agora_platform::no_console_window(&mut cmd);
    cmd.arg("--version");
    let (status, bytes) = output_within(cmd, timeout, VERSION_MAX)?;
    status
        .success()
        .then(|| String::from_utf8_lossy(&bytes).trim().to_string())
}

/// Runs `cmd` (no input, no error output) and returns how it ended and the first `max` bytes of its output, when it ends within
/// `timeout`; output past `max` is read and dropped, so a program that writes a lot is neither stalled nor stored. F68: a program
/// that does not end in time (or a process it started that keeps its output open) is killed and `None` is returned, so the doctor
/// always returns: on Unix with its process group, on Windows the direct child is killed and reaped (`kill_process_tree`); the
/// caller's time limit holds either way.
fn output_within(
    mut cmd: Command,
    timeout: std::time::Duration,
    max: usize,
) -> Option<(std::process::ExitStatus, Vec<u8>)> {
    use std::process::Stdio;
    agora_platform::own_process_group(&mut cmd);
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + timeout;
    let (tx, rx) = std::sync::mpsc::channel();
    if let Some(mut stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut kept = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                match stdout.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let room = max.saturating_sub(kept.len());
                        kept.extend_from_slice(&buf[..n.min(room)]);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            let _ = tx.send(kept);
        });
    }
    // The output to its end first, then the exit; the child is reaped only once it has ended, so the kill below still names it.
    let ended = rx.recv_timeout(timeout).ok().and_then(|bytes| loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some((status, bytes)),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            _ => return None,
        }
    });
    if ended.is_none() {
        agora_platform::kill_process_tree(&mut child);
    }
    ended
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
    check(
        "data-dir",
        if writable(dir) {
            Status::Ok
        } else {
            Status::Fail
        },
        dir.display().to_string(),
    )
}

/// Whether a file can be made and written in `dir` (made if missing). F67: a name already taken (left by an earlier run) is passed
/// over for the next one (`.doctor-<pid>`, then `.doctor-<pid>-1` to `-15`), and only a probe this run made is removed. `create_new` never follows a link planted at the probe's name.
fn writable(dir: &Path) -> bool {
    use std::io::Write;
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let pid = std::process::id();
    for n in 0..16 {
        // The first name is the one earlier versions used, so a probe one of them left behind is passed over too.
        let probe = dir.join(if n == 0 {
            format!(".doctor-{pid}")
        } else {
            format!(".doctor-{pid}-{n}")
        });
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
        {
            Ok(mut f) => {
                let written = f.write_all(b"x").is_ok();
                drop(f);
                let _ = std::fs::remove_file(&probe);
                return written;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return false,
        }
    }
    false
}

/// Every agent kind the node offers (the built-in ones and the folder's, `crate::manifests`): where it came from (`$from`: `built-in`
/// or `folder`), the program looked for (`$program`) and whether it is found (`agent-installed`), not found (`agent-missing`) or found
/// without the right to run it (`agent-broken`). The agent's own version is not shown: no manifest has a version command yet. A folder file that cannot be read is named; its
/// built-in kind, if any, stays. A missing folder is fine: the built-in kinds still load. The ACP registry's kinds (ADR-0092) are one
/// line, `registry-kinds` (`$detail`: how many, `$ready`: how many find their program), not one per kind: there are dozens, most are
/// not installed, and a missing one is not a problem. A registry kind whose program is broken or unsafe still gets its own line.
/// #1133: the owner's `agents.registry` policy is said too: `registry-off` (all switched off) or `registry-denied` (`$detail`: how many
/// the deny list switches off); a kind switched off is not counted or checked.
fn manifests(
    dir: &Path,
    home: &Path,
    registry_policy: &crate::manifests::RegistryPolicy,
) -> Vec<Check> {
    let (loaded, problems) = crate::manifests::load(dir);
    let mut out = vec![check("manifests", Status::Ok, loaded.len().to_string())];
    let (mut registry, mut registry_ready, mut denied) = (0usize, 0usize, 0usize);
    for l in loaded {
        // By origin: a hand-made kind is never switched off by the registry policy, whatever its name.
        if l.source == crate::manifests::Source::Registry
            && registry_policy.refusal(&l.manifest.kind).is_some()
        {
            denied += 1;
            continue;
        }
        let program = crate::manifests::program(&l.manifest)
            .unwrap_or_default()
            .to_string();
        let found = if program.is_empty() {
            None
        } else {
            locate_in(home, &program)
        };
        // Found but not runnable is "broken": the program is there and starting it would fail (ADR-0087 rung 1). Nothing is run to
        // find out, so a program that starts and then fails its own checks is not caught here.
        let broken = found.as_ref().is_some_and(|f| !runnable(f.path()));
        let installed = found.is_some() && !broken;
        // A registry kind is counted, not listed, unless its program is there and cannot run, or is Agora's and not safe.
        let from_registry = l.source == crate::manifests::Source::Registry;
        if from_registry {
            registry += 1;
            registry_ready += usize::from(installed);
        }
        let mut c = check(
            if installed {
                "agent-installed"
            } else if broken {
                "agent-broken"
            } else {
                "agent-missing"
            },
            if installed { Status::Ok } else { Status::Warn },
            l.manifest.kind.clone(),
        );
        c.args = vec![
            ("program", program.clone()),
            ("from", l.source.name().to_string()),
        ];
        // Where it was found: `agora` (installed by Agora, with the version folder) or `you` (found on the PATH or named by a path).
        if let Some(f) = &found {
            let (by, version) = match f {
                Found::Agora(_, v) => ("agora", v.clone()),
                Found::You(_) => ("you", String::new()),
            };
            c.args.push(("by", by.to_string()));
            c.args.push(("version", version));
        }
        if !from_registry || broken {
            out.push(c);
        }
        if !program.is_empty() && install_unsafe(home, &program) {
            let mut u = check("agent-unsafe", Status::Warn, l.manifest.kind);
            u.args = vec![("program", program)];
            out.push(u);
        }
    }
    if registry > 0 {
        let mut c = check("registry-kinds", Status::Ok, registry.to_string());
        c.args = vec![("ready", registry_ready.to_string())];
        out.push(c);
    }
    if registry_policy.off {
        out.push(check("registry-off", Status::Ok, ""));
    } else if denied > 0 {
        out.push(check("registry-denied", Status::Ok, denied.to_string()));
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

    /// A home with no installed agents.
    fn none_home() -> PathBuf {
        std::env::temp_dir().join("agora-doctor-no-such-home")
    }

    /// What `agora agent install` leaves for `exe`, made by hand.
    fn installed_layout(home: &Path, version: &str, exe: &str) {
        let agents = home.join("agents");
        let vdir = agents.join("fake").join(version);
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::create_dir_all(agents.join("bin")).unwrap();
        std::fs::write(vdir.join(exe), "x").unwrap();
        for d in [&agents, &agents.join("bin"), &agents.join("fake"), &vdir] {
            agora_platform::restrict_to_owner(d).unwrap();
        }
        agora_platform::set_executable(&vdir.join(exe), true).unwrap();
        std::fs::write(agents.join("fake").join("current"), version).unwrap();
        std::fs::write(agents.join("fake").join("exe"), exe).unwrap();
        let relative = Path::new("..").join("fake").join(version).join(exe);
        agora_platform::install_launcher(&vdir.join(exe), &relative, &agents.join("bin").join(exe))
            .unwrap();
    }

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
            agora_home: none_home(),
        });
        assert_eq!(find(&checks, "version")[0].status, Status::Ok);
        assert_eq!(find(&checks, "data-dir")[0].status, Status::Ok);
        assert_eq!(find(&checks, "node")[0].status, Status::Warn);
        // No manifests folder: the built-in kinds still load.
        assert_eq!(find(&checks, "manifests")[0].status, Status::Ok);
        assert_eq!(
            find(&checks, "manifests")[0].detail,
            crate::manifests::load(Path::new("")).0.len().to_string()
        );
        // #1133: the owner's policy switches them off, all or some; doctor says so and counts what is left.
        let off = crate::manifests::RegistryPolicy {
            off: true,
            ..Default::default()
        };
        let checks_off = manifests(&d.join("manifests"), &none_home(), &off);
        assert_eq!(find(&checks_off, "registry-off").len(), 1);
        assert!(find(&checks_off, "registry-kinds").is_empty());
        let deny = crate::manifests::RegistryPolicy {
            off: false,
            deny: ["registry-gemini".to_string(), "registry-goose".to_string()].into(),
        };
        let checks_deny = manifests(&d.join("manifests"), &none_home(), &deny);
        assert_eq!(find(&checks_deny, "registry-denied")[0].detail, "2");
        assert_eq!(
            find(&checks_deny, "registry-kinds")[0]
                .detail
                .parse::<usize>()
                .unwrap()
                + 2,
            find(&checks, "registry-kinds")[0]
                .detail
                .parse::<usize>()
                .unwrap()
        );
        // The registry's kinds are one line, not a warning per kind that is not installed.
        let registry = find(&checks, "registry-kinds");
        assert_eq!(registry.len(), 1);
        assert_eq!(registry[0].status, Status::Ok);
        assert!(registry[0].detail.parse::<usize>().unwrap() >= 40);
        assert!(!checks
            .iter()
            .any(|c| c.detail.starts_with(crate::manifests::REGISTRY_PREFIX)));
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
            agora_home: none_home(),
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
            agora_home: none_home(),
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
            agora_home: none_home(),
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
        let checks = manifests(&dir, &none_home(), &Default::default());
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
        assert!(find(
            &manifests(&dir, &none_home(), &Default::default()),
            "manifest-older"
        )
        .is_empty());
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
        let checks = manifests(&m, &none_home(), &Default::default());
        assert_eq!(checks[0].id, "manifests");
        assert_eq!(
            checks[0].detail,
            (crate::manifests::load(Path::new("")).0.len() + 2).to_string()
        );
        let of = |kind: &str| checks.iter().find(|c| c.detail == kind).unwrap();
        assert_eq!(of("demo").id, "agent-installed");
        assert_eq!(
            of("demo").args,
            [
                ("program", present.to_string()),
                ("from", "folder".into()),
                ("by", "you".into()),
                ("version", String::new())
            ]
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
        assert_eq!(
            of(&manifests(&m, &none_home(), &Default::default())),
            "agent-broken"
        );
        agora_platform::set_executable(&prog, true).unwrap();
        assert_eq!(
            of(&manifests(&m, &none_home(), &Default::default())),
            "agent-installed"
        );
        std::fs::remove_file(&prog).unwrap();
        assert_eq!(
            of(&manifests(&m, &none_home(), &Default::default())),
            "agent-missing"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_program_agora_installed_comes_first_and_is_told_apart_from_the_users_own() {
        let d = tmp("installed-by");
        let m = d.join("m");
        let home = d.join("home");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(
            m.join("a.toml"),
            MANIFEST.replace("LAUNCH", "installed-by-agora-xyz"),
        )
        .unwrap();
        let of = |checks: Vec<Check>| checks.into_iter().find(|c| c.detail == "demo").unwrap();
        // Nothing installed and nothing on PATH.
        assert_eq!(
            of(manifests(&m, &home, &Default::default())).id,
            "agent-missing"
        );
        installed_layout(&home, "1.2.3", "installed-by-agora-xyz");
        let c = of(manifests(&m, &home, &Default::default()));
        assert_eq!(c.id, "agent-installed");
        assert!(c.args.contains(&("by", "agora".to_string())));
        assert!(c.args.contains(&("version", "1.2.3".to_string())));
        // The installed one is the one to start, by its full path; another name is left to the PATH.
        let start = start_program(&home, "installed-by-agora-xyz");
        assert!(
            Path::new(&start).is_absolute() && Path::new(&start).is_file(),
            "{start}"
        );
        assert_eq!(start_program(&home, "other-xyz"), "other-xyz");
        assert_eq!(
            start_program(&none_home(), "installed-by-agora-xyz"),
            "installed-by-agora-xyz"
        );
        // The installed copy made unsafe: not used, and doctor says why.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = home.join("agents");
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
            let checks = manifests(&m, &home, &Default::default());
            assert_eq!(of(checks.clone()).id, "agent-missing");
            assert_eq!(find(&checks, "agent-unsafe").len(), 1);
            assert_eq!(
                start_program(&home, "installed-by-agora-xyz"),
                "installed-by-agora-xyz"
            );
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        // A program found only through a full path is "installed by you".
        let own = d.join("own-agent");
        std::fs::write(&own, "#!/bin/sh\n").unwrap();
        agora_platform::set_executable(&own, true).unwrap();
        std::fs::write(
            m.join("a.toml"),
            // Forward slashes: a Windows path's backslashes are escapes inside a TOML string.
            MANIFEST.replace("LAUNCH", &own.display().to_string().replace('\\', "/")),
        )
        .unwrap();
        let c = of(manifests(&m, &home, &Default::default()));
        assert_eq!(c.id, "agent-installed");
        assert!(c.args.contains(&("by", "you".to_string())));
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

    #[test]
    #[cfg(unix)]
    fn a_path_file_without_the_right_to_run_is_passed_over_for_a_later_runnable_one() {
        // F66: the launch skips the first copy, so the doctor must not call the program broken.
        let d = tmp("path-order");
        let (a, b) = (d.join("a"), d.join("b"));
        for dir in [&a, &b] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("f66-agent"), "#!/bin/sh\n").unwrap();
        }
        agora_platform::set_executable(&a.join("f66-agent"), false).unwrap();
        agora_platform::set_executable(&b.join("f66-agent"), true).unwrap();
        let paths = std::env::join_paths([&a, &b]).unwrap();
        assert_eq!(search(&paths, "f66-agent"), Some(b.join("f66-agent")));
        // None can run: the first file is named, so the doctor reports it broken rather than missing.
        agora_platform::set_executable(&b.join("f66-agent"), false).unwrap();
        assert_eq!(search(&paths, "f66-agent"), Some(a.join("f66-agent")));
        assert_eq!(search(&paths, "f66-none"), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_left_at_the_probe_name_neither_fails_the_data_dir_nor_is_removed() {
        // F67: a writable folder is reported writable, and a file this run did not make is kept.
        let d = tmp("stale-probe");
        // The first name tried, which is the only one the old code tried: there it failed the check and removed the file.
        let stale = d.join(format!(".doctor-{}", std::process::id()));
        std::fs::write(&stale, "not doctor's").unwrap();
        assert_eq!(data_dir(&d).status, Status::Ok);
        assert_eq!(std::fs::read_to_string(&stale).unwrap(), "not doctor's");
        // Only the stale file is left: the run's own probe is gone.
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    #[cfg(unix)]
    fn a_version_probe_that_does_not_end_is_killed_and_counts_as_not_working() {
        // F68: fake programs only. One that answers, one that never ends, one whose background child keeps the output open.
        let d = tmp("version-timeout");
        let script = |name: &str, body: &str| {
            let p = d.join(name);
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            agora_platform::set_executable(&p, true).unwrap();
            p.display().to_string()
        };
        let short = std::time::Duration::from_millis(500);
        let good = script("good", "echo 'fake 1.0'");
        assert_eq!(version_of(&good, short).as_deref(), Some("fake 1.0"));
        let failing = script("failing", "echo 'fake 1.0'; exit 3");
        assert_eq!(version_of(&failing, short), None);
        for (name, body) in [
            ("stuck", "echo $$ > \"$0.pid\"; exec sleep 600"),
            (
                "holder",
                "sh -c 'echo $$ > \"$0.pid\"; exec sleep 600' \"$0\" &",
            ),
        ] {
            let p = script(name, body);
            let began = std::time::Instant::now();
            assert_eq!(version_of(&p, short), None, "{name}");
            assert!(
                began.elapsed() < std::time::Duration::from_secs(5),
                "{name}"
            );
            let pid = std::fs::read_to_string(format!("{p}.pid")).unwrap();
            let gone = (0..200).any(|_| {
                let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid.trim()));
                // Gone, or a zombie no longer running (its parent reaps it).
                let dead = stat.map_or(true, |s| s.contains(") Z "));
                if !dead {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                dead
            });
            assert!(gone || !cfg!(target_os = "linux"), "{name} still running");
        }
        // Review of #921: the output kept is bounded, and a program that writes more than that is drained, not stalled.
        let chatty = script(
            "chatty",
            "echo 'fake 2.0'; head -c 3000000 /dev/zero | tr '\\0' x",
        );
        let v = version_of(&chatty, std::time::Duration::from_secs(5)).unwrap_or_default();
        assert!(
            v.starts_with("fake 2.0") && v.len() <= VERSION_MAX,
            "{}",
            v.len()
        );
        // The identity's `git config` reads are time-limited the same way: one that does not end reads as not set.
        let stuck = script("stuck-config", "exec sleep 600");
        let answering = script("answering-config", "printf 'Someone\\0'");
        let began = std::time::Instant::now();
        assert!(identity_missing_within(
            |_| Some(Command::new(&stuck)),
            short
        ));
        assert!(began.elapsed() < std::time::Duration::from_secs(5));
        assert!(!identity_missing_within(
            |_| Some(Command::new(&answering)),
            short
        ));
        let _ = std::fs::remove_dir_all(&d);
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
