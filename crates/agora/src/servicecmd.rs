//! `agora service install|remove|status`: start this machine's Agora when the user logs in (ADR-0062 item 6).
//!
//! One service per role the machine runs (`machine.toml`: the hub, the node), each running that role's supervisor in the foreground
//! (`agora supervise <role> --home <folder> --log`): the supervisor restarts the role and writes its log, the system only starts it at
//! login (and, where it can tell a signal from an exit, starts a supervisor that a signal killed again). Registration is per user,
//! never with administrator rights; the per-system code is in `agora_platform` (`service_install`, `service_remove`,
//! `service_status`). Before registering, the command says what it registers.
//!
//! `install` is idempotent: a role whose service is registered already is left as it is, and the command says so. It starts the role
//! now only when nothing holds it already. `remove` acts only on a registered service: it stops the role through its lock whoever
//! started it (the service, `agora`, or a supervisor started by hand), exactly as `agora down` does, and then unregisters the service.
//! Without a registered service it changes nothing (a role started by hand is stopped with `agora down`).

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

use agora_i18n::{key, Arg};
use agora_platform::{ServiceSpec, ServiceState};

use crate::frontdoor::{self, Machine, Role};
use crate::{home, Say};

/// The service names start with the binary's own name (`agora-node`): the one place the product name comes from here.
const NAME_PREFIX: &str = env!("CARGO_BIN_NAME");

fn service_name(role: Role) -> String {
    format!("{NAME_PREFIX}-{}", role.name())
}

/// The roles this machine runs, the hub first.
fn roles(m: &Machine) -> Vec<Role> {
    [Role::Hub, Role::Node]
        .into_iter()
        .filter(|r| m.has(r.name()))
        .collect()
}

fn spec(say: &Say, h: &Path, role: Role) -> std::io::Result<ServiceSpec> {
    // The service runs from no particular folder: a relative Agora folder would name another one there.
    let h = &std::path::absolute(h)?;
    let program = std::env::current_exe()?;
    let args: Vec<OsString> = vec![
        "supervise".into(),
        role.name().into(),
        "--home".into(),
        h.as_os_str().to_owned(),
        "--log".into(),
    ];
    // Until the login environment is captured (ADR-0062 item 5), the role gets this shell's PATH, so the agents it finds are the ones
    // this shell finds.
    let env = std::env::var_os("PATH")
        .map(|p| vec![("PATH".to_string(), p)])
        .unwrap_or_default();
    Ok(ServiceSpec {
        name: service_name(role),
        description: say.text(
            key!("service-description"),
            &[("role", role.label(say).into())],
        ),
        program,
        args,
        env,
        work_dir: h.to_path_buf(),
        restart_supervisor_on_failure: true,
    })
}

/// The command line a person would type for `spec`, for the message before registering.
fn shown(spec: &ServiceSpec) -> String {
    std::iter::once(spec.program.as_os_str())
        .chain(spec.args.iter().map(OsString::as_os_str))
        .map(|a| {
            let a = a.to_string_lossy();
            if a.contains(' ') {
                format!("\"{a}\"")
            } else {
                a.into_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `agora service install|remove|status`.
pub fn service(say: &Say, op: &str) -> ExitCode {
    if !agora_platform::SERVICES_SUPPORTED {
        say.err(key!("service-unsupported"), &[]);
        return ExitCode::from(1);
    }
    let h = home();
    let Some(m) = frontdoor::load(&h) else {
        say.err(key!("frontdoor-not-set-up"), &[]);
        return ExitCode::from(1);
    };
    match op {
        "install" => install(say, &h, &m),
        "remove" => remove(say, &h, &m),
        _ => status(say, &h, &m),
    }
}

pub(crate) fn install(say: &Say, h: &Path, m: &Machine) -> ExitCode {
    for d in ["run", "logs"] {
        if let Err(e) = std::fs::create_dir_all(h.join(d)) {
            say.err(
                key!("frontdoor-failed"),
                &[("reason", e.to_string().into())],
            );
            return ExitCode::from(1);
        }
    }
    let mut code = ExitCode::SUCCESS;
    for role in roles(m) {
        let label: Arg = role.label(say).into();
        let spec = match spec(say, h, role) {
            Ok(s) => s,
            Err(e) => {
                say.err(
                    key!("frontdoor-failed"),
                    &[("reason", e.to_string().into())],
                );
                return ExitCode::from(1);
            }
        };
        // A second `install` changes nothing and says so (to register it anew, `remove` first).
        match agora_platform::service_status(&spec.name) {
            Ok(ServiceState::Installed { .. }) => {
                say.out(
                    key!("service-already"),
                    &[("role", label), ("name", spec.name.clone().into())],
                );
                continue;
            }
            Ok(ServiceState::NotInstalled) => {}
            Err(e) => {
                say.err(
                    key!("service-status-failed"),
                    &[
                        ("name", spec.name.clone().into()),
                        ("reason", e.to_string().into()),
                    ],
                );
                code = ExitCode::from(1);
                continue;
            }
        }
        // What the system would change on the way (`%` expands in a Windows task) or cannot carry: refused with the path named.
        if let Some(bad) = std::iter::once(spec.program.as_os_str())
            .chain(spec.args.iter().map(OsString::as_os_str))
            .find(|a| a.to_str().is_none_or(|t| t.contains('%')))
        {
            say.err(
                key!("service-bad-path"),
                &[("path", bad.to_string_lossy().into_owned().into())],
            );
            code = ExitCode::from(1);
            continue;
        }
        say.out(
            key!("service-registering"),
            &[
                ("role", label.clone()),
                ("name", spec.name.clone().into()),
                ("command", shown(&spec).into()),
            ],
        );
        if let Err(e) = agora_platform::service_install(&spec) {
            say.err(
                key!("service-install-failed"),
                &[
                    ("name", spec.name.clone().into()),
                    ("reason", e.to_string().into()),
                ],
            );
            code = ExitCode::from(1);
            continue;
        }
        // Registered. Started now only when nothing holds the role already (a supervisor started now would only refuse the lock).
        if let Some(l) = frontdoor::holder(h, role) {
            say.out(
                key!("service-installed-running"),
                &[
                    ("role", label),
                    ("name", spec.name.clone().into()),
                    ("pid", i64::from(l.pid).into()),
                ],
            );
            continue;
        }
        match agora_platform::service_start(&spec.name) {
            Ok(()) => say.out(
                key!("service-installed"),
                &[("role", label), ("name", spec.name.clone().into())],
            ),
            Err(e) => {
                say.err(
                    key!("service-installed-not-started"),
                    &[
                        ("role", label),
                        ("name", spec.name.clone().into()),
                        ("reason", e.to_string().into()),
                    ],
                );
                code = ExitCode::from(1);
            }
        }
    }
    code
}

fn remove(say: &Say, h: &Path, m: &Machine) -> ExitCode {
    let mut code = ExitCode::SUCCESS;
    // The node first, then the hub, as `agora down`. Ending a Windows task ends only its window host, so each role is stopped through
    // its lock before its service is removed.
    for role in roles(m).into_iter().rev() {
        let name = service_name(role);
        let label: Arg = role.label(say).into();
        // Only a registered service is removed, and only then is its role stopped: `remove` on a machine without one leaves a role
        // started by hand alone (`agora down` stops that).
        match agora_platform::service_status(&name) {
            Ok(ServiceState::NotInstalled) => {
                say.out(key!("service-none"), &[("role", label)]);
                continue;
            }
            Ok(ServiceState::Installed { .. }) => {}
            Err(e) => {
                say.err(
                    key!("service-status-failed"),
                    &[("name", name.into()), ("reason", e.to_string().into())],
                );
                code = ExitCode::from(1);
                continue;
            }
        }
        frontdoor::stop_role(say, h, Some(m), role, &mut code);
        match agora_platform::service_remove(&name) {
            Ok(true) => say.out(
                key!("service-removed"),
                &[("role", label), ("name", name.into())],
            ),
            Ok(false) => say.out(key!("service-none"), &[("role", label)]),
            Err(e) => {
                say.err(
                    key!("service-remove-failed"),
                    &[("name", name.into()), ("reason", e.to_string().into())],
                );
                code = ExitCode::from(1);
            }
        }
    }
    code
}

fn status(say: &Say, h: &Path, m: &Machine) -> ExitCode {
    let mut code = ExitCode::SUCCESS;
    for role in roles(m) {
        let name = service_name(role);
        let label: Arg = role.label(say).into();
        match agora_platform::service_status(&name) {
            Ok(ServiceState::Installed { enabled: true }) => say.out(
                key!("service-status-installed"),
                &[("role", label), ("name", name.into())],
            ),
            Ok(ServiceState::Installed { enabled: false }) => say.out(
                key!("service-status-disabled"),
                &[("role", label), ("name", name.into())],
            ),
            Ok(ServiceState::NotInstalled) => {
                say.out(key!("service-status-not-installed"), &[("role", label)])
            }
            Err(e) => {
                say.err(
                    key!("service-status-failed"),
                    &[("name", name.into()), ("reason", e.to_string().into())],
                );
                code = ExitCode::from(1);
            }
        }
        // "Is it running?" is the lock's answer, whoever started it (the service, `agora`, a person).
        match frontdoor::holder(h, role) {
            Some(l) => {
                let _ = std::io::Write::write_fmt(
                    &mut std::io::stdout(),
                    format_args!(
                        "  {}\n",
                        say.text(
                            key!("service-status-running"),
                            &[("pid", i64::from(l.pid).into())]
                        )
                    ),
                );
            }
            None => {
                let _ = std::io::Write::write_fmt(
                    &mut std::io::stdout(),
                    format_args!("  {}\n", say.text(key!("service-status-stopped"), &[])),
                );
            }
        }
    }
    code
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn each_role_has_its_own_service_named_from_the_binary() {
        assert_eq!(service_name(Role::Node), format!("{NAME_PREFIX}-node"));
        assert_eq!(service_name(Role::Hub), format!("{NAME_PREFIX}-hub"));
        assert_eq!(NAME_PREFIX, "agora");
    }

    #[test]
    fn the_service_runs_the_supervisor_in_the_foreground_with_its_folder_and_log() {
        let say = Say(agora_i18n::Catalog::for_locale("en"));
        let h = std::env::temp_dir().join("agora home");
        let s = spec(&say, &h, Role::Node).unwrap();
        assert!(s.work_dir.is_absolute());
        // A relative Agora folder is made absolute: the service runs from no particular folder.
        let rel = spec(&say, Path::new("rel-home"), Role::Node).unwrap();
        assert!(rel.work_dir.is_absolute() && rel.work_dir.ends_with("rel-home"));
        assert!(Path::new(&rel.args[3]).is_absolute());
        let args: Vec<String> = s
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            ["supervise", "node", "--home", &h.to_string_lossy(), "--log"]
        );
        assert_eq!(s.work_dir, h);
        assert!(s.restart_supervisor_on_failure);
        assert!(shown(&s).contains(&format!("\"{}\"", h.display())));
    }
}
