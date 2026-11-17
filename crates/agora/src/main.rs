//! The single Agora binary: `agora hub`, `agora node`, `agora tui`, and CLI subcommands.
//!
//! This build implements the node-local commands: `agora node run`, `agora agent start|stop|list`, and
//! `agora terminal attach`. Each CLI command is one protocol command sent over the node's control socket
//! (`agora_node::control`). All sentences come from `locales/` through `agora_i18n::Catalog`; the process
//! prints only ids, paths and codes of its own.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use agora_i18n::{key, Arg, Catalog};
use agora_node::control::{Client, KIND_TERM_DATA, KIND_TERM_EXIT, KIND_TERM_SNAPSHOT};
use agora_node::daemon::{Node, NodeConfig};
use agora_node::supervisor::NodeLimits;
use agora_protocol::v1::{
    self, command_ok::Result as Ok_, command_request::Command as Cmd, param_value::Value,
};

/// Ctrl-] detaches from an attached terminal (like telnet), since everything else is the agent's input.
const DETACH: u8 = 0x1d;
const TERMINAL_STREAM: u32 = 3;

mod adaptercmd;
mod certs;
mod frontdoor;
mod hubcli;
mod joincmd;
mod meshbridge;
mod paircmd;
mod panelcmd;
mod remotecmd;
mod secretcmd;
mod servicecmd;
mod supervise;
mod tailscale;
mod trustcli;
mod tuicmd;
mod update;

pub(crate) struct Say(Catalog);

impl Say {
    fn text(&self, id: &str, args: &[(&str, Arg)]) -> String {
        self.0.render(id, args).unwrap_or_else(|| id.to_string())
    }

    fn out(&self, id: &str, args: &[(&str, Arg)]) {
        // A reader that went away (a closed pipe) must not take a running hub or node down with a panic.
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), "{}", self.text(id, args));
    }

    fn err(&self, id: &str, args: &[(&str, Arg)]) {
        eprintln!("{}", self.text(id, args));
    }

    /// A refused request whose `field` names something the person can act on has a sentence of its own
    /// (`cli-invalid-<field>`, dots as dashes), so it does not read like a protocol bug; the code stays the same.
    fn field_reason(&self, e: &v1::ErrorInfo, args: &[(&str, Arg)]) -> Option<String> {
        if e.code != "protocol.invalid_message" {
            return None;
        }
        let Some(Value::Str(field)) = e.params.get("field").and_then(|v| v.value.as_ref()) else {
            return None;
        };
        self.0
            .render(&format!("cli-invalid-{}", field.replace('.', "-")), args)
    }

    fn remote_error(&self, e: &v1::ErrorInfo) {
        eprintln!("{}", self.remote_error_text(e));
    }

    fn remote_error_text(&self, e: &v1::ErrorInfo) -> String {
        let args: Vec<(&str, Arg)> = e
            .params
            .iter()
            .filter_map(|(k, v)| match &v.value {
                Some(Value::Str(s)) => Some((k.as_str(), Arg::Str(s.clone()))),
                Some(Value::Int(i)) => Some((k.as_str(), Arg::Num(*i))),
                None => None,
            })
            .collect();
        let message = self
            .field_reason(e, &args)
            .unwrap_or_else(|| self.text(&e.code.replace('.', "-"), &args));
        self.text(
            key!("cli-error"),
            &[
                ("code", e.code.as_str().into()),
                ("message", message.into()),
            ],
        )
    }
}

pub(crate) fn home() -> PathBuf {
    if let Some(h) = std::env::var_os("AGORA_HOME") {
        return PathBuf::from(h);
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(
            || PathBuf::from(".agora"),
            |h| PathBuf::from(h).join(".agora"),
        )
}

/// Options shared by the subcommands, plus leftover positional arguments.
#[derive(Default)]
pub(crate) struct Opts {
    socket: Option<PathBuf>,
    manifests: Option<PathBuf>,
    data: Option<PathBuf>,
    worktrees: Option<PathBuf>,
    cwd: Option<String>,
    separate_worktree: bool,
    accept_identity_change: bool,
    projects: Vec<PathBuf>,
    project: Option<String>,
    max_agents: Option<usize>,
    hub_dir: Option<PathBuf>,
    listen: Option<String>,
    web: Option<PathBuf>,
    keep: Option<usize>,
    names: Vec<String>,
    hub: Option<String>,
    ca: Option<PathBuf>,
    code: Option<String>,
    id: Option<String>,
    url: Option<String>,
    port: Option<u16>,
    ntfy: Option<String>,
    webhook: Option<String>,
    test: bool,
    yes: bool,
    unsigned: bool,
    release_url: Option<String>,
    release_key: Option<PathBuf>,
    version: Option<String>,
    check: bool,
    registry: Option<String>,
    role: Option<String>,
    since: Option<u64>,
    /// ADR-0067 rung 3: `agora remote tailscale --serve` publishes the hub with `tailscale serve` (on `--port`, 8443 unless said).
    serve: bool,
    /// ADR-0065: `agora pair open --minutes N`.
    minutes: Option<u64>,
    who: Option<String>,
    root_key: Option<PathBuf>,
    /// ADR-0061: `agora node trust --from-hub`, and the fingerprints `--yes` covers.
    from_hub: bool,
    fingerprints: Vec<String>,
    /// ADR-0061: also other people's devices (with their fingerprints), and replacing a key already trusted under the same id.
    include_others: bool,
    /// `agora supervise`: the front door that started it and holds the lock until it names the supervisor (`PID:START`).
    handoff: Option<String>,
    /// `agora supervise`: the Agora folder, for a service manager that cannot set `AGORA_HOME` (a Windows task).
    home: Option<PathBuf>,
    /// `agora supervise`: write to the role's log file itself (a service manager redirects nothing).
    log: bool,
    replace: bool,
    /// ADR-0073: `agora hub machines forget --old` and `--refuse-return`.
    old: bool,
    refuse_return: bool,
    /// `agora hub run --metrics`: the hub answers `GET /api/metrics` (owner only).
    metrics: bool,
    /// ADR-0076: `agora hub metrics-token create NAME --expires DAYS`.
    expires: Option<u64>,
    ui: Option<String>,
    /// ADR-0062 item 4: `agora --resume ask|auto [--save]` and `agora node run --resume auto`.
    resume: Option<String>,
    /// ADR-0066: `agora agent start KIND --setting id=value` (repeatable).
    settings: Vec<String>,
    save: bool,
    positional: Vec<String>,
}

fn parse(args: &[String]) -> Option<Opts> {
    let mut o = Opts::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--socket" => o.socket = Some(it.next()?.into()),
            "--manifests" => o.manifests = Some(it.next()?.into()),
            "--data" => o.data = Some(it.next()?.into()),
            "--cwd" => o.cwd = Some(it.next()?.clone()),
            "--worktree" => o.separate_worktree = true,
            "--accept-project-id-change" => o.accept_identity_change = true,
            "--worktrees" => o.worktrees = Some(it.next()?.into()),
            "--project" => {
                let v = it.next()?;
                o.projects.push(v.into());
                o.project = Some(v.clone());
            }
            "--max-agents" => o.max_agents = Some(it.next()?.parse().ok()?),
            "--hub-dir" => o.hub_dir = Some(it.next()?.into()),
            "--keep" => o.keep = Some(it.next()?.parse().ok()?),
            "--web" => o.web = Some(it.next()?.into()),
            "--listen" => o.listen = Some(it.next()?.clone()),
            "--name" => o.names.push(it.next()?.clone()),
            "--hub" => o.hub = Some(it.next()?.clone()),
            "--ca" => o.ca = Some(it.next()?.into()),
            "--code" => o.code = Some(it.next()?.clone()),
            "--id" => o.id = Some(it.next()?.clone()),
            "--port" => o.port = Some(it.next()?.parse().ok()?),
            "--url" => o.url = Some(it.next()?.clone()),
            "--ntfy" => o.ntfy = Some(it.next()?.clone()),
            "--webhook" => o.webhook = Some(it.next()?.clone()),
            "--test" => o.test = true,
            "--yes" => o.yes = true,
            "--metrics" => o.metrics = true,
            "--serve" => o.serve = true,
            "--expires" => o.expires = Some(it.next()?.parse().ok()?),
            "--old" => o.old = true,
            "--refuse-return" => o.refuse_return = true,
            "--unsigned" => o.unsigned = true,
            "--release-url" => o.release_url = Some(it.next()?.clone()),
            "--release-key" => o.release_key = Some(it.next()?.into()),
            "--version" => o.version = Some(it.next()?.clone()),
            "--check" => o.check = true,
            "--registry" => o.registry = Some(it.next()?.clone()),
            "--minutes" => o.minutes = Some(it.next()?.parse().ok()?),
            "--since" => o.since = it.next()?.parse().ok(),
            "--who" => o.who = Some(it.next()?.clone()),
            "--role" => o.role = Some(it.next()?.clone()),
            "--root-key" => o.root_key = Some(it.next()?.into()),
            "--from-hub" => o.from_hub = true,
            "--fingerprint" => o.fingerprints.push(it.next()?.clone()),
            "--include-others" => o.include_others = true,
            "--handoff" => o.handoff = Some(it.next()?.clone()),
            "--home" => o.home = Some(it.next()?.into()),
            "--log" => o.log = true,
            "--replace" => o.replace = true,
            "--ui" => o.ui = Some(it.next()?.clone()),
            "--resume" => o.resume = Some(it.next()?.clone()),
            "--setting" => o.settings.push(it.next()?.clone()),
            "--save" => o.save = true,
            s if s.starts_with("--") => return None,
            _ => o.positional.push(a.clone()),
        }
    }
    Some(o)
}

pub(crate) fn socket_path(o: &Opts) -> PathBuf {
    o.socket.clone().unwrap_or_else(|| home().join("node.sock"))
}

/// `agora acp-bridge -- AGENT ARGS...`: runs an ACP agent and speaks the adapter protocol on this process's stdio (L2, the node
/// starts it; see `agora-adapters::acp`).
fn acp_bridge(args: &[String]) -> ExitCode {
    let agent = args.strip_prefix(&["--".to_string()]).unwrap_or(args);
    if agent.is_empty() {
        return ExitCode::from(2);
    }
    match agora_adapters::acp::run(agent, std::io::stdin(), std::io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(1),
    }
}

fn main() -> ExitCode {
    // A role started by its supervisor ends with it (ADR-0062 item 8); first, before any thread exists, as it changes the environment.
    agora_platform::end_with_supervisor();
    if std::env::args().nth(1).as_deref() == Some("acp-bridge") {
        let args: Vec<String> = std::env::args().skip(2).collect();
        return acp_bridge(&args);
    }
    let say = Say(Catalog::for_locale(&agora_i18n::catalog::locale_from_env()));
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(
        args.first().map(String::as_str),
        Some("--help" | "-h" | "help")
    ) {
        say.out(key!("cli-usage"), &[]);
        return ExitCode::SUCCESS;
    }
    if matches!(
        args.first().map(String::as_str),
        Some("--version" | "-V" | "version")
    ) {
        say.out(
            key!("cli-version"),
            &[("version", env!("CARGO_PKG_VERSION").into())],
        );
        return ExitCode::SUCCESS;
    }
    let words: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .take(2)
        .take_while(|w| !w.starts_with("--"))
        .collect();
    let rest = args.get(words.len()..).unwrap_or(&[]);
    let Some(opts) = parse(rest) else {
        say.err(key!("cli-bad-arguments"), &[]);
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    };
    match words.as_slice() {
        // ADR-0062: plain `agora` brings this machine's Agora up and opens it.
        [] => frontdoor::front_door(&say, &opts),
        ["web"] => frontdoor::web(&say),
        ["status"] => frontdoor::status(&say, &opts.data),
        ["down"] => frontdoor::down(&say),
        ["service", op @ ("install" | "remove" | "status")] => servicecmd::service(&say, op),
        ["node", "run"] => node_run(&say, &opts),
        // ADR-0065: the owner's switch for new machines, and a new machine asking to join.
        ["pair", op @ ("open" | "close" | "status")] => paircmd::pair(&say, &opts, op),
        ["join"] => joincmd::join(&say, &opts, opts.positional.first().map(String::as_str)),
        ["join", address] => joincmd::join(&say, &opts, Some(address)),
        // Started by the front door, not by people: not in the usage page.
        ["supervise", role] => supervise::supervise(
            &say,
            role,
            &supervise::Start {
                handoff: opts.handoff.as_deref(),
                home: opts.home.clone(),
                log: opts.log,
                resume: opts.resume.as_deref(),
            },
        ),
        ["agent", "start"] => agent_start(&say, &opts),
        ["project", "init"] => project_init(&say, &opts),
        ["agent", "stop"] => agent_stop(&say, &opts),
        ["agent", "prompt"] => agent_prompt(&say, &opts),
        ["agent", "list"] => agent_list(&say, &opts),
        ["agent", "kinds"] => agent_kinds(&say, &opts),
        ["terminal", "attach"] => terminal_attach(&say, &opts),
        ["panel"] => panelcmd::panel(&say),
        ["doctor"] => doctor(&say, &opts),
        ["tui"] => tuicmd::tui(&say, &opts),
        ["update"] => update::update(&say, &opts),
        ["adapter", "install"] => adaptercmd::install(&say, &opts),
        ["adapter", "list"] => adaptercmd::list(&say, &opts),
        ["adapter", "remove"] => adaptercmd::remove(&say, &opts),
        ["adapter", "update"] => adaptercmd::update(&say, &opts),
        ["remote", op @ ("tailscale" | "status" | "off")] => remotecmd::remote(&say, &opts, op),
        ["password", op @ ("set" | "clear" | "status")] => hubcli::password(&say, &opts, op),
        ["hub", "run"] => hubcli::hub_run(&say, &opts),
        // A machine that does not run the hub (no hub folder) gets that said, not the database error these commands would hit.
        // The commands that create the hub folder themselves (`code`, `login`, `user`, `prices`, `ssh`, `notify`, `backup`,
        // `restore`) are not listed.
        ["hub", "fingerprint" | "audit" | "devices" | "machines"]
            if !hubcli::hub_dir(&opts).is_dir() =>
        {
            hubcli::not_here(&say, &opts)
        }
        ["hub", "code"] => hubcli::hub_code(&say, &opts),
        ["hub", "fingerprint"] => hubcli::hub_fingerprint(&say, &opts),
        ["hub", "login"] => hubcli::hub_login(&say, &opts),
        ["hub", "user"] => hubcli::hub_user(&say, &opts),
        ["hub", "metrics-token"] => hubcli::hub_metrics_token(&say, &opts),
        ["hub", "audit"] => hubcli::hub_audit(&say, &opts),
        ["hub", "devices"] => hubcli::hub_devices(&say, &opts),
        ["hub", "machines"] => hubcli::hub_machines(&say, &opts),
        ["hub", "ssh"] => hubcli::hub_ssh(&say, &opts),
        ["hub", "prices"] => hubcli::hub_prices(&say, &opts),
        ["hub", "notify"] => hubcli::hub_notify(&say, &opts),
        ["hub", "backup"] => hubcli::hub_backup(&say, &opts),
        ["hub", "restore"] => hubcli::hub_restore(&say, &opts),
        ["node", "enroll"] => hubcli::node_enroll(&say, &opts),
        ["node", "trust"] => trustcli::node_trust(&say, &opts),
        ["node", "untrust"] => trustcli::node_untrust(&say, &opts),
        ["node", "secret"] => secretcmd::secret(&say, &opts),
        ["node", "stdio"] => bridge_stdio(&say, &socket_path(&opts)),
        ["mesh", "stdio"] => match &opts.socket {
            // An agent's MCP client starts this as its mesh server; the node told it which socket is its own.
            Some(path) => match meshbridge::mesh_stdio(path) {
                Ok(code) => code,
                Err(e) => {
                    say_unreachable(&say, path, &e);
                    ExitCode::from(1)
                }
            },
            None => {
                say.err(key!("cli-bad-arguments"), &[]);
                ExitCode::from(2)
            }
        },
        _ => {
            say.err(key!("cli-usage"), &[]);
            ExitCode::from(2)
        }
    }
}

/// `agora node stdio` and `agora mesh stdio`: carry a local socket over this process's stdin/stdout.
/// - `node stdio` bridges the node's framed protocol, so a remote hub can reach a node over
///   `ssh host agora node stdio` with no open port (ADR-0011). The node, its agents and its policy are unchanged, and
///   it never needs the hub.
/// - `mesh stdio --socket PATH` is what an agent's MCP client starts as its mesh server: it carries the MCP
///   conversation to that agent's own endpoint on the node.
fn bridge_stdio(say: &Say, path: &std::path::Path) -> ExitCode {
    // The control token is presented by the client (ADR-0082), so the hub that talks through this bridge needs nothing of it.
    let sock = match Client::connect(path).map(Client::into_stream) {
        Ok(sock) => sock,
        Err(e) => {
            say_unreachable(say, path, &e);
            return ExitCode::from(1);
        }
    };
    let Ok(mut from_node) = sock.try_clone() else {
        return ExitCode::from(1);
    };
    // node → stdout. Ends when the node closes the connection or the reader below shuts the socket.
    let out = std::thread::spawn(move || {
        let mut stdout = std::io::stdout().lock();
        let mut buf = [0u8; 16 * 1024];
        while let Ok(n) = from_node.read(&mut buf) {
            if n == 0
                || stdout
                    .write_all(&buf[..n])
                    .and_then(|()| stdout.flush())
                    .is_err()
            {
                break;
            }
        }
    });
    // stdin → node. EOF means the remote side hung up: close the socket so the thread above wakes.
    let mut to_node = sock;
    let mut stdin = std::io::stdin().lock();
    let mut buf = [0u8; 16 * 1024];
    while let Ok(n) = stdin.read(&mut buf) {
        if n == 0 || to_node.write_all(&buf[..n]).is_err() {
            break;
        }
    }
    let _ = to_node.shutdown();
    let _ = out.join();
    ExitCode::SUCCESS
}

/// `agora doctor`: checks that explain most "it does not work" reports.
fn doctor(say: &Say, o: &Opts) -> ExitCode {
    use agora_node::doctor::{healthy, run, Inputs, Status};
    let h = home();
    let mut checks = run(&Inputs {
        data_dir: o.data.clone().unwrap_or_else(|| h.join("data")),
        socket: socket_path(o),
        manifests_dir: o.manifests.clone().unwrap_or_else(|| h.join("manifests")),
    });
    // Certificates last 90 days and nothing renews them yet: say so before a machine stops connecting.
    checks.extend(certs::checks_now(&hubcli::link_dir(), &hubcli::hub_dir(o)));
    // ADR-0067 rung 3: how the hub is reached from outside this machine, and whether the screen password protects it.
    checks.extend(remotecmd::checks(o));
    for c in &checks {
        let sentence = match (c.id, c.status) {
            ("version", _) => key!("doctor-version"),
            ("pty", Status::Ok) => key!("doctor-pty"),
            ("pty", _) => key!("doctor-pty-unsupported"),
            ("local-sockets", Status::Ok) => key!("doctor-local-sockets"),
            ("local-sockets", _) => key!("doctor-local-sockets-unsupported"),
            ("sandbox", Status::Ok) => key!("doctor-sandbox"),
            ("sandbox", _) => key!("doctor-sandbox-unsupported"),
            ("sockets-profile", _) => key!("doctor-sockets-profile"),
            ("sockets-abstract", _) => key!("doctor-sockets-abstract"),
            ("sockets-none", _) => key!("doctor-sockets-none"),
            ("env-scrubbed", _) => key!("doctor-env-scrubbed"),
            ("env-passed", _) => key!("doctor-env-passed"),
            ("git-identity-missing", _) => key!("doctor-git-identity-missing"),
            ("control-token-reachable", _) => key!("doctor-control-token-reachable"),
            ("git", Status::Ok) => key!("doctor-git"),
            ("git", _) => key!("doctor-git-missing"),
            ("data-dir", Status::Ok) => key!("doctor-data-dir"),
            ("data-dir", _) => key!("doctor-data-dir-unwritable"),
            ("node", Status::Ok) => key!("doctor-node"),
            ("node", _) => key!("doctor-node-not-running"),
            ("manifests", _) => key!("doctor-manifests"),
            ("agent-installed", _) => key!("doctor-agent-installed"),
            ("agent-missing", _) => key!("doctor-agent-missing"),
            ("manifest-older", _) => key!("doctor-manifest-older"),
            ("policy-path-dropped", _) => key!("doctor-policy-path-dropped"),
            ("policy-path-missing", _) => key!("doctor-policy-path-missing"),
            ("policy-restart", _) => key!("doctor-policy-restart"),
            ("policy-changed", _) => key!("doctor-policy-changed"),
            ("signed-commands", _) => key!("doctor-signed-commands"),
            ("signed-commands-team", _) => key!("doctor-signed-commands-team"),
            ("signed-commands-none", _) => key!("doctor-signed-commands-none"),
            ("cert-node", Status::Ok) => key!("doctor-cert-node"),
            ("cert-node", Status::Warn) => key!("doctor-cert-node-soon"),
            ("cert-node", _) if c.detail == certs::UNUSABLE => key!("doctor-cert-node-unusable"),
            ("cert-node", _) => key!("doctor-cert-node-expired"),
            ("cert-hub", Status::Ok) => key!("doctor-cert-hub"),
            ("cert-hub", Status::Warn) => key!("doctor-cert-hub-soon"),
            ("cert-hub", _) if c.detail == certs::UNUSABLE => key!("doctor-cert-hub-unusable"),
            ("cert-hub", _) => key!("doctor-cert-hub-expired"),
            ("devices", _) => key!("doctor-devices"),
            ("devices-none", _) => key!("doctor-devices-none"),
            ("skip-permissions", _) => key!("doctor-skip-permissions"),
            ("policy-problems", _) => key!("doctor-policy-problems"),
            ("remote-password-set", _) => key!("doctor-remote-password-set"),
            ("remote-password-missing", _) => key!("doctor-remote-password-missing"),
            ("remote-password-missing-tailnet", _) => {
                key!("doctor-remote-password-missing-tailnet")
            }
            ("remote-public-listen", _) => key!("doctor-remote-public-listen"),
            ("remote-funnel", _) => key!("doctor-remote-funnel"),
            ("remote-tailnet", _) => key!("doctor-remote-tailnet"),
            ("remote-tailnet-unreachable", _) => key!("doctor-remote-tailnet-unreachable"),
            ("remote-names-missing", _) => key!("doctor-remote-names-missing"),
            ("credential", _) => key!("doctor-credential"),
            ("credential-none", _) => key!("doctor-credential-none"),
            ("credential-unlisted", _) => key!("doctor-credential-unlisted"),
            ("credential-refused", _) => key!("doctor-credential-refused"),
            ("credential-malformed", _) => key!("doctor-credential-malformed"),
            _ => key!("doctor-manifest-invalid"),
        };
        let detail: Arg = match (c.id, c.detail.parse::<i64>()) {
            ("manifests" | "devices" | "policy-problems" | "cert-node" | "cert-hub", Ok(n)) => {
                Arg::Num(n)
            }
            _ => c.detail.as_str().into(),
        };
        let mut args = vec![("detail", detail)];
        args.extend(c.args.iter().map(|(k, v)| (*k, Arg::from(v.as_str()))));
        let text = say.text(sentence, &args);
        let status = say.text(
            match c.status {
                Status::Ok => key!("doctor-status-ok"),
                Status::Warn => key!("doctor-status-warn"),
                Status::Fail => key!("doctor-status-fail"),
            },
            &[],
        );
        say.out(
            key!("doctor-line"),
            &[("status", status.into()), ("text", text.into())],
        );
    }
    if healthy(&checks) {
        say.out(key!("doctor-summary-ok"), &[]);
        ExitCode::SUCCESS
    } else {
        say.err(key!("doctor-summary-problems"), &[]);
        ExitCode::from(1)
    }
}

fn node_run(say: &Say, o: &Opts) -> ExitCode {
    // A crash can leave folder permissions for a confined agent behind (Windows); nothing is running yet, so undo them.
    agora_platform::recover_sandbox_leftovers();
    if !matches!(o.resume.as_deref(), None | Some("auto" | "ask")) {
        say.err(key!("cli-bad-arguments"), &[]);
        return ExitCode::from(2);
    }
    let h = home();
    // A machine enrolled with a hub reports under the id the hub's certificate names.
    let link = hubcli::load_link(socket_path(o));
    let cfg = NodeConfig {
        // `agora` passes `--resume auto` unless the owner said no (ADR-0062 item 4).
        resume_all: o.resume.as_deref() == Some("auto"),
        node_id: link
            .as_ref()
            .map_or_else(|| "local".to_string(), |(id, _)| id.clone()),
        socket: socket_path(o),
        data_dir: o.data.clone().unwrap_or_else(|| h.join("data")),
        worktree_root: o.worktrees.clone().unwrap_or_else(|| h.join("worktrees")),
        manifests_dir: o.manifests.clone().unwrap_or_else(|| h.join("manifests")),
        projects: o.projects.clone(),
        limits: NodeLimits {
            max_agents: o.max_agents,
            ..NodeLimits::default()
        },
    };
    if let Some(parent) = cfg.socket.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let node = match Node::start(&cfg) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    // An earlier run's link state is never read as this run's; a node without a hub has none.
    agora_node::link_state::LinkState::clear(&cfg.data_dir);
    for (path, reason) in &node.manifest_problems {
        eprintln!("{}: {reason}", path.display());
    }
    // ADR-0059: a policy line the node did not understand forbids starting agents that do not ask, until it is fixed.
    for line in &node.policy_problems {
        say.err(
            key!("cli-node-policy-not-understood"),
            &[("line", line.clone().into())],
        );
    }
    // Events recorded under another name (before enrollment, as `local`) are never sent to the hub, which would refuse them: kept
    // on disk under a new name, and said here so nothing is lost silently.
    if let Some(s) = &node.events_set_aside {
        say.err(
            key!("cli-node-events-set-aside"),
            &[
                ("node_id", cfg.node_id.clone().into()),
                ("old_id", s.old_id.clone().into()),
                ("path", s.path.display().to_string().into()),
            ],
        );
    }
    say.out(
        key!("cli-node-listening"),
        &[("socket", cfg.socket.display().to_string().into())],
    );
    // Connect to the hub in the background when enrolled. Agents never depend on it: events wait in the buffer.
    let _uplink = link.and_then(|(_, mut l)| {
        // Messages for agents on other machines go through the hub connection (ADR-0031).
        let out = agora_node::uplink_client::MeshOut::new();
        node.mesh.set_remote(out.clone());
        l.mesh_out = Some(out);
        l.grants = Some(Arc::clone(&node.grants));
        l.mesh = Some(Arc::clone(&node.mesh));
        l.devices = Some(Arc::clone(&node.devices));
        l.state_dir = Some(cfg.data_dir.clone());
        agora_node::uplink_client::spawn(Arc::clone(&node.events), l).ok()
    });
    // Run until the process is stopped; agents die with it and the socket file is replaced on next start.
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

fn connect(say: &Say, o: &Opts) -> Result<Client, ExitCode> {
    let path = socket_path(o);
    Client::connect(&path).map_err(|e| {
        say_unreachable(say, &path, &e);
        ExitCode::from(1)
    })
}

/// Why the node's control socket could not be opened: refused (another user, or a confined agent: the socket is owner-only and a
/// sandbox may not reach it, T16) is not the same as no node at all, and saying "no node is listening" for it hides a refusal.
fn unreachable_key(e: &std::io::Error) -> &'static str {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        key!("cli-node-refused")
    } else {
        key!("cli-node-not-running")
    }
}

fn say_unreachable(say: &Say, path: &std::path::Path, e: &std::io::Error) {
    say.err(
        unreachable_key(e),
        &[("socket", path.display().to_string().into())],
    );
}

/// Send one command; print the error (and return failure) if the node refuses it.
fn call(say: &Say, c: &mut Client, cmd: Cmd) -> Result<v1::CommandOk, ExitCode> {
    match c.call(cmd) {
        Ok(Ok(ok)) => Ok(ok),
        // ADR-0082: no readable control token (a confined agent): say that, not "you are not signed in".
        Ok(Err(e)) if e.code == agora_protocol::ErrorCode::AuthUnauthenticated.as_str() => {
            say.err(key!("cli-node-needs-token"), &[]);
            Err(ExitCode::from(1))
        }
        Ok(Err(e)) => {
            say.remote_error(&e);
            Err(ExitCode::from(1))
        }
        Err(e) => {
            eprintln!("{e}");
            Err(ExitCode::from(1))
        }
    }
}

fn selected_path(o: &Opts) -> String {
    // A registered name is resolved on the node, even if the caller has a different same-named folder.
    if o.cwd.is_none() {
        if let Some(project) = &o.project {
            if !project.contains(['/', '\\']) {
                return project.clone();
            }
        }
    }
    let path = o
        .cwd
        .clone()
        .or_else(|| o.project.clone())
        .unwrap_or_else(|| {
            std::env::current_dir().map_or(String::new(), |p| p.to_string_lossy().into_owned())
        });
    std::fs::canonicalize(&path).map_or(path, |p| p.to_string_lossy().into_owned())
}
fn inspect_directory(
    say: &Say,
    c: &mut Client,
    path: &str,
    create: bool,
) -> Result<v1::DirectoryInfo, ExitCode> {
    match call(
        say,
        c,
        Cmd::DirectoryInspect(v1::DirectoryInspect {
            path: path.into(),
            create_project_id: create,
            authorization: None,
        }),
    )? {
        v1::CommandOk {
            result: Some(Ok_::DirectoryInfo(i)),
        } => Ok(i),
        _ => Err(ExitCode::from(1)),
    }
}
fn project_init(say: &Say, o: &Opts) -> ExitCode {
    let mut c = match connect(say, o) {
        Ok(c) => c,
        Err(e) => return e,
    };
    match inspect_directory(say, &mut c, &selected_path(o), true) {
        Ok(i) => {
            say.out(
                key!("cli-project-identity"),
                &[("id", i.project_id.into()), ("path", i.path.into())],
            );
            ExitCode::SUCCESS
        }
        Err(e) => e,
    }
}

fn agent_start(say: &Say, o: &Opts) -> ExitCode {
    let Some(kind) = o.positional.first() else {
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    };
    // ADR-0066: `id=value` each; the node checks the id and the value against its manifest.
    let Some(settings) = o
        .settings
        .iter()
        .map(|s| {
            s.split_once('=').map(|(id, value)| v1::LaunchSetting {
                id: id.to_string(),
                value: value.to_string(),
            })
        })
        .collect::<Option<Vec<_>>>()
    else {
        say.err(key!("cli-bad-arguments"), &[]);
        return ExitCode::from(2);
    };
    let mut c = match connect(say, o) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let path = selected_path(o);
    let info = match inspect_directory(say, &mut c, &path, false) {
        Ok(i) => i,
        Err(e) => return e,
    };
    let cmd = Cmd::AgentLaunch(v1::AgentLaunch {
        kind: kind.clone(),
        path: info.path,
        project_id: info.project_id,
        separate_worktree: o.separate_worktree,
        accept_identity_change: o.accept_identity_change,
        authorization: None,
        launch_options: Vec::new(),
        confirmed_launch_options: Vec::new(),
        settings,
    });
    match call(say, &mut c, cmd) {
        Ok(v1::CommandOk {
            result: Some(Ok_::AgentLaunch(r)),
        }) => {
            say.out(
                key!("cli-agent-started"),
                &[("agent_id", r.agent_id.into()), ("worktree", r.path.into())],
            );
            ExitCode::SUCCESS
        }
        Ok(_) => ExitCode::from(1),
        Err(code) => code,
    }
}

/// `agora agent list`: one line per agent, columns separated by tabs (id, kind, state, running or stopped).
fn agent_list(say: &Say, o: &Opts) -> ExitCode {
    let mut c = match connect(say, o) {
        Ok(c) => c,
        Err(code) => return code,
    };
    match call(say, &mut c, Cmd::AgentList(v1::AgentList::default())) {
        Ok(v1::CommandOk {
            result: Some(Ok_::AgentList(list)),
        }) => {
            if list.agents.is_empty() {
                say.out(key!("cli-agent-list-empty"), &[]);
            }
            for a in &list.agents {
                let state = v1::AgentState::try_from(a.state).map_or("unknown", |s| match s {
                    v1::AgentState::Working => "working",
                    v1::AgentState::Blocked => "blocked",
                    v1::AgentState::Idle => "idle",
                    v1::AgentState::Done => "done",
                    v1::AgentState::Unspecified => "unknown",
                });
                println!(
                    "{}\t{}\t{}\t{}",
                    a.agent_id,
                    a.kind,
                    state,
                    if a.running { "running" } else { "stopped" }
                );
            }
            ExitCode::SUCCESS
        }
        Ok(_) => ExitCode::from(1),
        Err(code) => code,
    }
}

/// `agora agent kinds`: the agents this node can start, and whether each is installed.
fn agent_kinds(say: &Say, o: &Opts) -> ExitCode {
    let mut c = match connect(say, o) {
        Ok(c) => c,
        Err(code) => return code,
    };
    match call(say, &mut c, Cmd::AgentKinds(v1::AgentKinds {})) {
        Ok(v1::CommandOk {
            result: Some(Ok_::AgentKinds(list)),
        }) => {
            if list.kinds.is_empty() {
                say.out(key!("cli-agent-kinds-empty"), &[]);
            }
            for k in &list.kinds {
                println!(
                    "{}\t{}\t{}",
                    k.kind,
                    k.display_name,
                    if k.installed { "installed" } else { "missing" }
                );
            }
            ExitCode::SUCCESS
        }
        Ok(_) => ExitCode::from(1),
        Err(code) => code,
    }
}

fn agent_stop(say: &Say, o: &Opts) -> ExitCode {
    let Some(id) = o.positional.first() else {
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    };
    let mut c = match connect(say, o) {
        Ok(c) => c,
        Err(code) => return code,
    };
    match call(
        say,
        &mut c,
        Cmd::AgentStop(v1::AgentStop {
            agent_id: id.clone(),
            stop_helpers: false,
        }),
    ) {
        Ok(_) => {
            say.out(
                key!("cli-agent-stopped"),
                &[("agent_id", id.as_str().into())],
            );
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

fn agent_prompt(say: &Say, o: &Opts) -> ExitCode {
    let (Some(id), text) = (
        o.positional.first(),
        o.positional
            .iter()
            .skip(1)
            .cloned()
            .collect::<Vec<_>>()
            .join(" "),
    ) else {
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    };
    if text.is_empty() {
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    }
    let mut c = match connect(say, o) {
        Ok(c) => c,
        Err(code) => return code,
    };
    match call(
        say,
        &mut c,
        Cmd::AgentPrompt(v1::AgentPrompt {
            agent_id: id.clone(),
            text,
        }),
    ) {
        Ok(_) => {
            say.out(
                key!("cli-agent-prompted"),
                &[("agent_id", id.as_str().into())],
            );
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

fn terminal_attach(say: &Say, o: &Opts) -> ExitCode {
    let Some(agent) = o.positional.first() else {
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    };
    let terminal_id = format!("t-{agent}");
    let mut c = match connect(say, o) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let Ok(raw) = agora_platform::RawMode::enable() else {
        say.err(key!("cli-attach-needs-terminal"), &[]);
        return ExitCode::from(1);
    };
    // Match the node's terminal to ours, then follow it.
    if let Some((cols, rows)) = agora_platform::terminal_size() {
        let _ = c.call(Cmd::TerminalResize(v1::TerminalResize {
            terminal_id: terminal_id.clone(),
            cols: u32::from(cols),
            rows: u32::from(rows),
        }));
    }
    let subscribed = c.call_on(
        TERMINAL_STREAM,
        Cmd::TerminalSubscribe(v1::TerminalSubscribe {
            terminal_id: terminal_id.clone(),
            format: "terminal.raw".into(),
        }),
    );
    match subscribed {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            drop(raw);
            say.remote_error(&e);
            return ExitCode::from(1);
        }
        Err(e) => {
            drop(raw);
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    }

    let detach = Arc::new(AtomicBool::new(false));
    if let Ok(writer) = c.writer() {
        let (detach, terminal_id) = (Arc::clone(&detach), terminal_id.clone());
        // Detached thread: it ends with the process (a blocked stdin read cannot be interrupted).
        let _ = std::thread::Builder::new()
            .name("agora-attach-stdin".into())
            .spawn(move || {
                let mut buf = [0u8; 4096];
                let mut stdin = std::io::stdin().lock();
                while let Ok(n) = stdin.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let cut = buf[..n].iter().position(|b| *b == DETACH);
                    let data = buf[..cut.unwrap_or(n)].to_vec();
                    if !data.is_empty()
                        && writer
                            .send(Cmd::TerminalInput(v1::TerminalInput {
                                terminal_id: terminal_id.clone(),
                                data,
                            }))
                            .is_err()
                    {
                        break;
                    }
                    if cut.is_some() {
                        detach.store(true, Ordering::Release);
                        break;
                    }
                }
                detach.store(true, Ordering::Release);
            });
    }

    // Follow the window being resized: the loop wakes every 100 ms anyway, so compare the size then (no signal
    // handling, so it works the same wherever terminals do).
    let resizer = c.writer().ok();
    let mut last_size = agora_platform::terminal_size();
    let mut stdout = std::io::stdout();
    loop {
        let now = agora_platform::terminal_size();
        if now != last_size {
            last_size = now;
            if let (Some(w), Some((cols, rows))) = (&resizer, now) {
                let _ = w.send(Cmd::TerminalResize(v1::TerminalResize {
                    terminal_id: terminal_id.clone(),
                    cols: u32::from(cols),
                    rows: u32::from(rows),
                }));
            }
        }
        match c.read_frame(Some(Duration::from_millis(100))) {
            Ok(Some(f)) if f.stream_id == TERMINAL_STREAM => match f.kind {
                KIND_TERM_SNAPSHOT | KIND_TERM_DATA => {
                    let _ = stdout.write_all(&f.payload).and_then(|()| stdout.flush());
                }
                KIND_TERM_EXIT => break,
                _ => {}
            },
            Ok(Some(_)) => {}
            Ok(None) => {}
            // The node went away: nothing left to show.
            Err(_) => break,
        }
        if detach.load(Ordering::Acquire) {
            break;
        }
    }
    drop(raw);
    println!();
    say.out(
        key!("cli-terminal-detached"),
        &[("agent_id", agent.as_str().into())],
    );
    ExitCode::SUCCESS
}

#[cfg(test)]
mod unreachable_tests {
    use super::*;
    use std::io::{Error, ErrorKind};

    #[test]
    fn a_refused_control_socket_is_not_worded_as_a_stopped_node() {
        assert_eq!(
            unreachable_key(&Error::from(ErrorKind::PermissionDenied)),
            "cli-node-refused"
        );
        assert_eq!(
            unreachable_key(&Error::from(ErrorKind::NotFound)),
            "cli-node-not-running"
        );
        assert_eq!(
            unreachable_key(&Error::from(ErrorKind::ConnectionRefused)),
            "cli-node-not-running"
        );
        for c in [Catalog::english(), Catalog::turkish()] {
            let t = c.render("cli-node-refused", &[("socket", "S".into())]);
            assert!(t.as_deref().is_some_and(|t| t.contains('S')), "{t:?}");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod remote_error_tests {
    use super::*;
    use agora_protocol::{Error, ErrorCode};

    #[test]
    fn a_changed_project_identity_says_what_to_do_instead_of_reading_like_a_protocol_bug() {
        let e = Error::new(ErrorCode::ProtocolInvalidMessage)
            .with("field", "directory.identity_changed")
            .to_info();
        let en = Say(Catalog::english()).remote_error_text(&e);
        assert!(en.starts_with("Error protocol.invalid_message: "), "{en}");
        assert!(en.contains("--accept-project-id-change"), "{en}");
        assert!(!en.contains("not valid"), "{en}");
        let tr = Say(Catalog::turkish()).remote_error_text(&e);
        assert!(tr.contains("--accept-project-id-change"), "{tr}");
        assert!(tr.contains("proje kimliği"), "{tr}");
        // A field without a sentence of its own keeps the generic one.
        let other = Error::new(ErrorCode::ProtocolInvalidMessage)
            .with("field", "directory.project_id")
            .to_info();
        let en = Say(Catalog::english()).remote_error_text(&other);
        assert!(en.contains("field: directory.project_id"), "{en}");
    }
}
