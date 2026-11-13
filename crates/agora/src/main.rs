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
mod hubcli;
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

    fn remote_error(&self, e: &v1::ErrorInfo) {
        let args: Vec<(&str, Arg)> = e
            .params
            .iter()
            .filter_map(|(k, v)| match &v.value {
                Some(Value::Str(s)) => Some((k.as_str(), Arg::Str(s.clone()))),
                Some(Value::Int(i)) => Some((k.as_str(), Arg::Num(*i))),
                None => None,
            })
            .collect();
        let message = self.text(&e.code.replace('.', "-"), &args);
        self.err(
            key!("cli-error"),
            &[
                ("code", e.code.as_str().into()),
                ("message", message.into()),
            ],
        );
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
    who: Option<String>,
    root_key: Option<PathBuf>,
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
            "--unsigned" => o.unsigned = true,
            "--release-url" => o.release_url = Some(it.next()?.clone()),
            "--release-key" => o.release_key = Some(it.next()?.into()),
            "--version" => o.version = Some(it.next()?.clone()),
            "--check" => o.check = true,
            "--registry" => o.registry = Some(it.next()?.clone()),
            "--since" => o.since = it.next()?.parse().ok(),
            "--who" => o.who = Some(it.next()?.clone()),
            "--role" => o.role = Some(it.next()?.clone()),
            "--root-key" => o.root_key = Some(it.next()?.into()),
            s if s.starts_with("--") => return None,
            _ => o.positional.push(a.clone()),
        }
    }
    Some(o)
}

fn socket_path(o: &Opts) -> PathBuf {
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
    if std::env::args().nth(1).as_deref() == Some("acp-bridge") {
        let args: Vec<String> = std::env::args().skip(2).collect();
        return acp_bridge(&args);
    }
    let say = Say(Catalog::for_locale(&agora_i18n::catalog::locale_from_env()));
    let args: Vec<String> = std::env::args().skip(1).collect();
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
        ["node", "run"] => node_run(&say, &opts),
        ["agent", "start"] => agent_start(&say, &opts),
        ["project", "init"] => project_init(&say, &opts),
        ["agent", "stop"] => agent_stop(&say, &opts),
        ["agent", "prompt"] => agent_prompt(&say, &opts),
        ["agent", "list"] => agent_list(&say, &opts),
        ["agent", "kinds"] => agent_kinds(&say, &opts),
        ["terminal", "attach"] => terminal_attach(&say, &opts),
        ["panel"] => hubcli::panel(&say),
        ["doctor"] => doctor(&say, &opts),
        ["tui"] => tuicmd::tui(&say, &opts),
        ["update"] => update::update(&say, &opts),
        ["adapter", "install"] => adaptercmd::install(&say, &opts),
        ["adapter", "list"] => adaptercmd::list(&say, &opts),
        ["adapter", "remove"] => adaptercmd::remove(&say, &opts),
        ["adapter", "update"] => adaptercmd::update(&say, &opts),
        ["hub", "run"] => hubcli::hub_run(&say, &opts),
        ["hub", "code"] => hubcli::hub_code(&say, &opts),
        ["hub", "fingerprint"] => hubcli::hub_fingerprint(&say, &opts),
        ["hub", "login"] => hubcli::hub_login(&say, &opts),
        ["hub", "user"] => hubcli::hub_user(&say, &opts),
        ["hub", "audit"] => hubcli::hub_audit(&say, &opts),
        ["hub", "ssh"] => hubcli::hub_ssh(&say, &opts),
        ["hub", "prices"] => hubcli::hub_prices(&say, &opts),
        ["hub", "notify"] => hubcli::hub_notify(&say, &opts),
        ["hub", "backup"] => hubcli::hub_backup(&say, &opts),
        ["hub", "restore"] => hubcli::hub_restore(&say, &opts),
        ["node", "enroll"] => hubcli::node_enroll(&say, &opts),
        ["node", "trust"] => node_trust(&say, &opts),
        ["node", "stdio"] => bridge_stdio(&say, &socket_path(&opts)),
        ["mesh", "stdio"] => match &opts.socket {
            // An agent's MCP client starts this as its mesh server; the node told it which socket is its own.
            Some(path) => bridge_stdio(&say, path),
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
    let Ok(sock) = agora_platform::local_connect(path) else {
        say.err(
            key!("cli-node-not-running"),
            &[("socket", path.display().to_string().into())],
        );
        return ExitCode::from(1);
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
    let checks = run(&Inputs {
        data_dir: o.data.clone().unwrap_or_else(|| h.join("data")),
        socket: socket_path(o),
        manifests_dir: o.manifests.clone().unwrap_or_else(|| h.join("manifests")),
    });
    for c in &checks {
        let sentence = match (c.id, c.status) {
            ("version", _) => key!("doctor-version"),
            ("pty", Status::Ok) => key!("doctor-pty"),
            ("pty", _) => key!("doctor-pty-unsupported"),
            ("local-sockets", Status::Ok) => key!("doctor-local-sockets"),
            ("local-sockets", _) => key!("doctor-local-sockets-unsupported"),
            ("sandbox", Status::Ok) => key!("doctor-sandbox"),
            ("sandbox", _) => key!("doctor-sandbox-unsupported"),
            ("git", Status::Ok) => key!("doctor-git"),
            ("git", _) => key!("doctor-git-missing"),
            ("data-dir", Status::Ok) => key!("doctor-data-dir"),
            ("data-dir", _) => key!("doctor-data-dir-unwritable"),
            ("node", Status::Ok) => key!("doctor-node"),
            ("node", _) => key!("doctor-node-not-running"),
            ("manifests", _) => key!("doctor-manifests"),
            ("agent-installed", _) => key!("doctor-agent-installed"),
            ("agent-missing", _) => key!("doctor-agent-missing"),
            ("policy-path-dropped", _) => key!("doctor-policy-path-dropped"),
            ("policy-path-missing", _) => key!("doctor-policy-path-missing"),
            _ => key!("doctor-manifest-invalid"),
        };
        let detail: Arg = match (c.id, c.detail.parse::<i64>()) {
            ("manifests", Ok(n)) => Arg::Num(n),
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

/// `agora node trust DEVICE_ID PUBLIC_KEY_HEX`: lets a device answer approvals on this node. Run by the owner on the
/// machine itself; nothing a hub or client sends can do this.
fn node_trust(say: &Say, o: &Opts) -> ExitCode {
    let (Some(id), Some(key)) = (o.positional.first(), o.positional.get(1)) else {
        say.err(key!("cli-bad-arguments"), &[]);
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    };
    let dir = o.data.clone().unwrap_or_else(|| home().join("data"));
    let result = std::fs::create_dir_all(&dir)
        .map_err(|e| e.to_string())
        .and_then(|()| {
            agora_node::approvals::trust_device(
                &dir.join(agora_node::approvals::TRUSTED_DEVICES_FILE),
                id,
                key,
            )
        });
    match result {
        Ok(()) => {
            say.out(key!("cli-trusted"), &[("id", id.clone().into())]);
            ExitCode::SUCCESS
        }
        Err(e) => {
            say.err(key!("cli-trust-failed"), &[("reason", e.into())]);
            ExitCode::from(1)
        }
    }
}

fn node_run(say: &Say, o: &Opts) -> ExitCode {
    // A crash can leave folder permissions for a confined agent behind (Windows); nothing is running yet, so undo them.
    agora_platform::recover_sandbox_leftovers();
    let h = home();
    // A machine enrolled with a hub reports under the id the hub's certificate names.
    let link = hubcli::load_link(socket_path(o));
    let cfg = NodeConfig {
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
    for (path, reason) in &node.manifest_problems {
        eprintln!("{}: {reason}", path.display());
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
        agora_node::uplink_client::spawn(Arc::clone(&node.events), l).ok()
    });
    // Run until the process is stopped; agents die with it and the socket file is replaced on next start.
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

fn connect(say: &Say, o: &Opts) -> Result<Client, ExitCode> {
    let path = socket_path(o);
    Client::connect(&path).map_err(|_| {
        say.err(
            key!("cli-node-not-running"),
            &[("socket", path.display().to_string().into())],
        );
        ExitCode::from(1)
    })
}

/// Send one command; print the error (and return failure) if the node refuses it.
fn call(say: &Say, c: &mut Client, cmd: Cmd) -> Result<v1::CommandOk, ExitCode> {
    match c.call(cmd) {
        Ok(Ok(ok)) => Ok(ok),
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
