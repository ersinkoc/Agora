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
    let p = Path::new(program);
    if p.components().count() > 1 {
        return p.is_file();
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        dir.join(program).is_file()
            || (cfg!(windows) && dir.join(format!("{program}.exe")).is_file())
    })
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
    let sandbox = crate::daemon::read_policy(&policy_file).sandbox;
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
    }

    out.push(match Command::new("git").arg("--version").output() {
        Ok(o) if o.status.success() => check(
            "git",
            Status::Ok,
            String::from_utf8_lossy(&o.stdout).trim().to_string(),
        ),
        _ => check("git", Status::Fail, ""),
    });

    out.push(data_dir(&inputs.data_dir));

    out.push(
        if agora_platform::LOCAL_SOCKETS_SUPPORTED
            && agora_platform::local_connect(&inputs.socket).is_ok()
        {
            check("node", Status::Ok, inputs.socket.display().to_string())
        } else {
            check("node", Status::Warn, inputs.socket.display().to_string())
        },
    );

    out.extend(manifests(&inputs.manifests_dir));
    out
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
/// or `folder`), the program looked for (`$program`) and whether it is found. A folder file that cannot be read is named; its
/// built-in kind, if any, stays. A missing folder is fine: the built-in kinds still load.
fn manifests(dir: &Path) -> Vec<Check> {
    let (loaded, problems) = crate::manifests::load(dir);
    let mut out = vec![check("manifests", Status::Ok, loaded.len().to_string())];
    for l in loaded {
        let program = crate::manifests::program(&l.manifest)
            .unwrap_or_default()
            .to_string();
        let installed = !program.is_empty() && on_path(&program);
        let mut c = check(
            if installed {
                "agent-installed"
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
    out
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
            crate::manifests::BUILT_IN.len().to_string()
        );
        // Where terminals are unsupported (Windows for now) that is a reported problem, not a healthy machine.
        let git_ok = find(&checks, "git")[0].status == Status::Ok;
        assert_eq!(healthy(&checks), git_ok && agora_platform::PTY_SUPPORTED);
        let _ = std::fs::remove_dir_all(&d);
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
            (crate::manifests::BUILT_IN.len() + 2).to_string()
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
}
