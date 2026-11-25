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

/// Written to the user's own terminal before anything of the agent's (`terminal attach`): a kitty keyboard entry of our own on top
/// of the terminal's stack, so the flags the agent sets change only our entry, and detaching pops it and whatever the agent left
/// pushed above it ([`HostKeyboard`]); and the terminal's own alternate-scroll setting saved (XTSAVE `CSI ? 1007 s`), since the
/// repaint sets 1007 as the node's emulator has it (on) and terminals differ in their default.
const ATTACH_START: &[u8] = b"\x1b[>0u\x1b[?1007s";

/// Written to the user's own terminal on detach, after [`HostKeyboard::detach`] (the main screen, where our keyboard entry is, and
/// the pop of our entry and the agent's): the modes the node's repaint (ADR-0100) or the agent may have changed put back to a
/// terminal's defaults: mouse tracking and encodings, focus, bracketed paste, application cursor keys and keypad, insert and
/// newline off, autowrap on, alternate scroll off and then the terminal's own setting restored where it supports XTRESTORE
/// (`CSI ? 1007 r`; elsewhere off, xterm's default), a plain pen in ASCII, the cursor shown in the terminal's own shape.
const DETACH_RESET: &[u8] =
    b"\x1b[?1007l\x1b[?1007r\x1b[?9l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1015l\
\x1b[?1004l\x1b[?2004l\x1b[?1l\x1b>\x1b[4l\x1b[20l\x1b[?7h\x1b[0m\x1b(B\x0f\x1b[?25h\x1b[0 q";

/// The node's repaint as the user's own terminal gets it: without the kitty keyboard pop (`CSI < 32 u`) the node writes for a
/// window, which here would pop the user's terminal's own entries (a shell's or tmux's) below ours. The node writes it once, as
/// these exact bytes, after the drawing; nothing a program printed is in a repaint.
fn repaint_for_host(repaint: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    const POP: &[u8] = b"\x1b[<32u";
    match repaint.windows(POP.len()).position(|w| w == POP) {
        Some(at) => std::borrow::Cow::Owned([&repaint[..at], &repaint[at + POP.len()..]].concat()),
        None => std::borrow::Cow::Borrowed(repaint),
    }
}
const TERMINAL_STREAM: u32 = 3;

/// What `terminal attach` has to pop from the user's own terminal on detach: its own kitty keyboard entry and the entries the agent
/// pushed above it on the main screen and did not pop (`CSI > f u` adds one, `CSI < n u` removes n, at least 1). Pushes on the
/// alternate screen are not counted: the terminal keeps a stack per screen and the alternate one goes with `CSI ? 1049 l`. Reads the
/// bytes written to the terminal (the repaint and the live output), across frame boundaries; not a hot path (one client).
#[derive(Default)]
struct HostKeyboard {
    /// 0 ground, 1 after ESC, 2 inside a CSI sequence.
    state: u8,
    started: bool,
    private: u8,
    number: u32,
    digits: bool,
    first: Option<u32>,
    /// The sequence names 47, 1047 or 1049 (the alternate screen).
    screen: bool,
    alt: bool,
    pushed: u32,
    /// Our own entry is gone: the agent popped past it, or reset the terminal. Only what it pushed since is popped on detach.
    gone: bool,
}

impl HostKeyboard {
    fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            match (self.state, b) {
                (_, 0x1b) => self.state = 1,
                (0, _) => {}
                (1, b'[') => {
                    *self = Self {
                        state: 2,
                        alt: self.alt,
                        pushed: self.pushed,
                        gone: self.gone,
                        ..Self::default()
                    };
                }
                (1, b'c') => {
                    // A full reset empties the terminal's stacks.
                    self.state = 0;
                    self.alt = false;
                    self.pushed = 0;
                    self.gone = true;
                }
                (1, _) => self.state = 0,
                (_, 0x18 | 0x1a) => self.state = 0,
                (_, b'<' | b'=' | b'>' | b'?') if !self.started => {
                    self.started = true;
                    self.private = b;
                }
                (_, b'0'..=b'9') => {
                    self.started = true;
                    self.digits = true;
                    self.number = self
                        .number
                        .saturating_mul(10)
                        .saturating_add(u32::from(b - b'0'));
                }
                (_, b';' | b':') => {
                    self.started = true;
                    self.end_number();
                }
                (_, 0x40..=0x7e) => {
                    self.end_number();
                    self.finish(b);
                    self.state = 0;
                }
                _ => self.started = true,
            }
        }
    }

    fn end_number(&mut self) {
        if self.digits {
            self.first.get_or_insert(self.number);
            if self.private == b'?' && matches!(self.number, 47 | 1047 | 1049) {
                self.screen = true;
            }
        }
        self.number = 0;
        self.digits = false;
    }

    fn finish(&mut self, fin: u8) {
        match (self.private, fin) {
            (b'>', b'u') if !self.alt => self.pushed = self.pushed.saturating_add(1),
            (b'<', b'u') if !self.alt => {
                let n = self.first.unwrap_or(1).max(1);
                if n > self.pushed && !self.gone {
                    // Past the agent's own entries: ours goes too, and some of the terminal's below it.
                    self.gone = true;
                }
                self.pushed = self.pushed.saturating_sub(n);
            }
            (b'?', b'h') if self.screen => self.alt = true,
            (b'?', b'l') if self.screen => self.alt = false,
            _ => {}
        }
    }

    /// Back to the main screen if the agent left the terminal on the alternate one (only then: xterm and xterm.js restore the saved
    /// cursor on `CSI ? 1049 l` even on the main screen, which would put the next lines over the top of it), then the pop of what
    /// the agent left pushed on the main screen and of our own entry while it is still there.
    fn detach(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.alt {
            out.extend_from_slice(b"\x1b[?1049l");
        }
        let pop = self.pushed.saturating_add(u32::from(!self.gone));
        if pop > 0 {
            out.extend_from_slice(format!("\x1b[<{pop}u").as_bytes());
        }
        out
    }
}

mod adaptercmd;
mod agentcmd;
mod certs;
mod explaincmd;
mod frontdoor;
mod hold;
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
        // The sentence ends in a parameter from the other side (`adapter-agent_exited`'s `stderr_tail`), which may be empty or
        // end in a line break; the line printed ends at its text.
        self.text(
            key!("cli-error"),
            &[
                ("code", e.code.as_str().into()),
                ("message", message.trim().into()),
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
    /// ADR-0067 rung 4: `agora remote proxy --trust-header NAME --host PUBLIC.NAME [--host ...] [--proxy-addr IP ...]`.
    trust_header: Option<String>,
    hosts: Vec<String>,
    proxy_addrs: Vec<String>,
    /// ADR-0065: `agora pair open --minutes N`.
    minutes: Option<u64>,
    who: Option<String>,
    root_key: Option<PathBuf>,
    /// ADR-0087 rung 3: `agora agent install NAME --list FILE [--go-back]`.
    list: Option<PathBuf>,
    go_back: bool,
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
    /// `agora down --agents`: also stops the agents the terminal holder keeps running (ADR-0079).
    agents: bool,
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
    /// ADR-0060, #1137: `agora agent resume AGENT_ID --again`, Start again (a new session in the same folder) instead of Resume.
    again: bool,
    /// `agora agent explain KIND --file SCREEN [--size COLSxROWS]`: the captured screen, and the terminal size to replay it on.
    file: Option<PathBuf>,
    size: Option<String>,
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
            "--again" => o.again = true,
            "--file" => o.file = Some(it.next()?.into()),
            "--size" => o.size = Some(it.next()?.clone()),
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
            "--trust-header" => o.trust_header = Some(it.next()?.clone()),
            "--host" => o.hosts.push(it.next()?.clone()),
            "--proxy-addr" => o.proxy_addrs.push(it.next()?.clone()),
            "--expires" => o.expires = Some(it.next()?.parse().ok()?),
            "--old" => o.old = true,
            "--agents" => o.agents = true,
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
            "--list" => o.list = Some(it.next()?.into()),
            "--go-back" => o.go_back = true,
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

/// `agora acp-bridge [--runner-cwd DIR] -- AGENT ARGS...`: runs an ACP agent and speaks the adapter protocol on this process's stdio
/// (L2, the node starts it; see `agora-adapters::acp`). `--runner-cwd` starts the agent's command in `DIR` (#1150).
fn acp_bridge(args: &[String]) -> ExitCode {
    let (agent_cwd, args) = match args.split_first() {
        Some((flag, rest)) if flag == agora_adapters::acp::RUNNER_CWD_FLAG => {
            match rest.split_first() {
                Some((dir, rest)) if !dir.is_empty() => (Some(std::path::Path::new(dir)), rest),
                _ => return ExitCode::from(2),
            }
        }
        _ => (None, args),
    };
    let agent = args.strip_prefix(&["--".to_string()]).unwrap_or(args);
    if agent.is_empty() {
        return ExitCode::from(2);
    }
    match agora_adapters::acp::run_in(agent, agent_cwd, std::io::stdin(), std::io::stdout().lock())
    {
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
        ["down"] => frontdoor::down(&say, opts.agents),
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
        // ADR-0079: the terminal holder, started by the node (not by people, so not in the usage page).
        ["hold"] => hold::hold(&say, &opts),
        ["agent", "start"] => agent_start(&say, &opts),
        ["project", "init"] => project_init(&say, &opts),
        ["agent", "stop"] => agent_stop(&say, &opts),
        ["agent", "prompt"] => agent_prompt(&say, &opts),
        ["agent", "resume"] => agent_resume(&say, &opts),
        ["agent", "list"] => agent_list(&say, &opts),
        ["agent", "kinds"] => agent_kinds(&say, &opts),
        ["agent", "explain"] => explaincmd::explain(&say, &opts),
        ["agent", "install"] => agentcmd::install(&say, &opts),
        ["terminal", "attach"] => terminal_attach(&say, &opts),
        ["panel"] => panelcmd::panel(&say),
        ["doctor"] => doctor(&say, &opts),
        ["tui"] => tuicmd::tui(&say, &opts),
        ["update"] => update::update(&say, &opts),
        ["adapter", "install"] => adaptercmd::install(&say, &opts),
        ["adapter", "list"] => adaptercmd::list(&say, &opts),
        ["adapter", "remove"] => adaptercmd::remove(&say, &opts),
        ["adapter", "update"] => adaptercmd::update(&say, &opts),
        ["remote", op @ ("tailscale" | "proxy" | "status" | "off")] => {
            remotecmd::remote(&say, &opts, op)
        }
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
        ["history", "purge"] => history_purge(&say, &opts),
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

/// ADR-0089 decision 3: `agora history purge` removes every chat transcript this machine's node keeps (`<data>/transcripts`). Files only:
/// a node that runs writes the turns that come after it again.
fn history_purge(say: &Say, opts: &Opts) -> ExitCode {
    let data = opts.data.clone().unwrap_or_else(|| home().join("data"));
    match agora_node::transcript::purge_all(&data) {
        Ok(()) => {
            say.out(key!("cli-history-purged"), &[]);
            ExitCode::SUCCESS
        }
        Err(_) => {
            say.err(key!("cli-history-purge-failed"), &[]);
            ExitCode::from(1)
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
        agora_home: agora_adapters::package::agora_home(),
    });
    // Certificates last 90 days and renew themselves (ADR-0085): the days left, and what a refused renewal needs.
    checks.extend(certs::checks_now(&hubcli::link_dir(), &hubcli::hub_dir(o)));
    // ADR-0067 rung 3: how the hub is reached from outside this machine, and whether the screen password protects it.
    checks.extend(remotecmd::checks(o));
    // ADR-0093: a holder that speaks only link version 1 cannot keep structured agents across a node restart.
    checks.extend(agora_node::doctor::holder_checks(&h));
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
            ("protected-path-reachable", _) => key!("doctor-protected-path-reachable"),
            ("git", Status::Ok) => key!("doctor-git"),
            ("git", _) => key!("doctor-git-missing"),
            ("data-dir", Status::Ok) => key!("doctor-data-dir"),
            ("data-dir", _) => key!("doctor-data-dir-unwritable"),
            ("node", Status::Ok) => key!("doctor-node"),
            ("node", _) => key!("doctor-node-not-running"),
            ("manifests", _) => key!("doctor-manifests"),
            ("agent-installed", _) => key!("doctor-agent-installed"),
            ("agent-missing", _) => key!("doctor-agent-missing"),
            ("agent-broken", _) => key!("doctor-agent-broken"),
            ("agent-unsafe", _) => key!("doctor-agent-unsafe"),
            ("registry-kinds", _) => key!("doctor-registry-kinds"),
            ("registry-off", _) => key!("doctor-registry-off"),
            ("registry-denied", _) => key!("doctor-registry-denied"),
            ("runner-folders-left", _) => key!("doctor-runner-folders-left"),
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
            ("cert-renew-join-again", _) => key!("doctor-cert-renew-join-again"),
            ("cert-renew-superseded", _) => key!("doctor-cert-renew-superseded"),
            ("cert-renew-rejected", _) => key!("doctor-cert-renew-rejected"),
            ("cert-renew-hub-old", _) => key!("doctor-cert-renew-hub-old"),
            ("cert-renew-pending", _) => key!("doctor-cert-renew-pending"),
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
            ("remote-proxy-on", _) => key!("doctor-remote-proxy-on"),
            ("remote-proxy-listen", _) => key!("doctor-remote-proxy-listen"),
            ("remote-proxy-damaged", _) => key!("doctor-remote-proxy-damaged"),
            ("holder-old", _) => key!("doctor-holder-old"),
            ("credential", _) => key!("doctor-credential"),
            ("credential-none", _) => key!("doctor-credential-none"),
            ("credential-unlisted", _) => key!("doctor-credential-unlisted"),
            ("credential-refused", _) => key!("doctor-credential-refused"),
            ("credential-malformed", _) => key!("doctor-credential-malformed"),
            _ => key!("doctor-manifest-invalid"),
        };
        let detail: Arg = match (c.id, c.detail.parse::<i64>()) {
            (
                "manifests"
                | "devices"
                | "policy-problems"
                | "cert-node"
                | "cert-hub"
                | "holder-old"
                | "registry-kinds"
                | "registry-denied"
                | "runner-folders-left",
                Ok(n),
            ) => Arg::Num(n),
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

/// How long a node's start tries again to read a link folder that is there but cannot be read (a volume still being mounted) before it
/// ends: longer than `agora_core::restart::RestartRule::default().refusal_ms`, so its supervisor takes the end for a crash and starts it
/// again later instead of giving up.
const LINK_UNREADABLE_RETRY: std::time::Duration = std::time::Duration::from_secs(15);

/// ADR-0104 O2: the longest `agora node` takes from a stop request (`SIGTERM`, `SIGINT`, `SIGHUP`; Unix) to its end. It stops cleanly
/// within it (the node's drain waits at most `agora_node::daemon::LOCK_WAIT` and `DRAIN_WAIT`, 3 s together, for the supervisor's lock
/// and the held adapters' readers, and `LOOP_WAIT`, 1 s, for the commands already running; the rest is local writes, and those come
/// first when the lock is busy), and ends anyway when it passes, whatever still runs. Under every wait for it: `agora down` waits 10 s for the
/// supervisor, which ends right after the node (`frontdoor::stop_role`, also used by `agora service remove`), and the systemd unit's
/// `TimeoutStopSec` is 25 s. Windows ends the node with `TerminateProcess`, which cannot be caught.
const NODE_STOP_DEADLINE: Duration = Duration::from_secs(5);

/// How often the stop watch looks for a caught stop request (the handler itself may only set a flag).
const STOP_POLL: Duration = Duration::from_millis(100);

/// ADR-0104 O2: from now on a stop request does not end `agora node` at once once its node is up (`up` set): what this returns says
/// it arrived, so the node stops cleanly, and the process ends anyway [`NODE_STOP_DEADLINE`] after it. Before the node is up there is
/// nothing to keep, and a request ends the process at once, as a second request always does. `None` where stop requests are not
/// caught (Windows) or the watch could not start: a request then ends the process at once, as before.
fn node_stop_watch(up: Arc<AtomicBool>) -> Option<std::sync::mpsc::Receiver<()>> {
    if !agora_platform::STOP_REQUESTS_CAUGHT {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    // The watch first: requests are caught only once something is there to act on them.
    std::thread::Builder::new()
        .name("agora-stop-watch".into())
        .spawn(move || loop {
            if agora_platform::stop_requested() {
                if !up.load(Ordering::Acquire) {
                    agora_platform::end_as_stopped();
                }
                let _ = tx.send(());
                std::thread::sleep(NODE_STOP_DEADLINE);
                // Operational log, not UI text. Never a panic (a closed or hung-up stderr): the end below must come.
                let _ = writeln!(
                    std::io::stderr(),
                    "node: not stopped {} s after the stop request; ending now",
                    NODE_STOP_DEADLINE.as_secs()
                );
                agora_platform::end_as_stopped();
            }
            std::thread::sleep(STOP_POLL);
        })
        .ok()?;
    agora_platform::catch_stop_requests_once().ok()?;
    Some(rx)
}

fn node_run(say: &Say, o: &Opts) -> ExitCode {
    // A crash can leave folder permissions for a confined agent behind (Windows); nothing is running yet, so undo them.
    agora_platform::recover_sandbox_leftovers();
    if !matches!(o.resume.as_deref(), None | Some("auto" | "ask")) {
        say.err(key!("cli-bad-arguments"), &[]);
        return ExitCode::from(2);
    }
    let h = home();
    // ADR-0104 O2: a stop request from here on ends the node cleanly (its agents in the holder keep running), within the deadline, once
    // the node is up; before, at once.
    let up = Arc::new(AtomicBool::new(false));
    let stop = node_stop_watch(Arc::clone(&up));
    // A machine enrolled with a hub reports under the id the hub's certificate names. A link folder another command held past every
    // working hold is not read (it may be half saved) and the node does not start with no link or half of one: it ends, and its
    // supervisor starts it again later (Codex review of F1031). A folder that cannot be read yet (a volume still being mounted) is tried
    // again for a while first: an end that soon after the start would be taken for a refusal, and the supervisor would not try again.
    let started = std::time::Instant::now();
    let link = loop {
        match hubcli::load_link(socket_path(o)) {
            Err(e)
                if e.kind() != std::io::ErrorKind::TimedOut
                    && started.elapsed() < LINK_UNREADABLE_RETRY =>
            {
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            r => break r,
        }
    };
    let link = match link {
        Ok(link) => link,
        Err(e) => {
            let dir = ("dir", hubcli::link_dir().display().to_string().into());
            if e.kind() == std::io::ErrorKind::TimedOut {
                say.err(key!("cli-link-busy"), &[dir]);
            } else {
                say.err(
                    key!("cli-link-unreadable"),
                    &[dir, ("reason", e.to_string().into())],
                );
            }
            return ExitCode::from(1);
        }
    };
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
    // ADR-0079: agents start through the terminal holder where there is one, so they outlive this node.
    let node = match Node::start_with_holder(&cfg, hold::setup_for_node(&h)) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    // ADR-0104 O2: from here a stop request is a clean stop.
    up.store(true, Ordering::Release);
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
    let mut _renewal = None;
    let _uplink = link.and_then(|(_, mut l)| {
        // ADR-0085: the certificate is renewed by the node itself when a third of its life is left, and the next connection uses the
        // new one. A failure here changes nothing for the agents or the connection that is up.
        let dir = hubcli::link_dir();
        let renewer = agora_node::link_renewal::Renewer::new(
            dir.clone(),
            || agora_hub::ca::new_device_request().ok(),
            move || {
                hubcli::hold_link(&dir).map(|held| Box::new(held) as Box<dyn std::any::Any + Send>)
            },
        );
        l.certs = Some(renewer.clone());
        _renewal = agora_node::link_renewal::spawn(renewer).ok();
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
    // Run until the process is stopped; the socket file is replaced on next start. ADR-0104 O2: on a stop request the node stops cleanly
    // (it records what the held adapters said and it had not recorded, writes the conversation, the usage and the ledger, and lets the
    // held agents go; no agent is ended) and the process then ends as the request would have ended it.
    let Some(stop) = stop else {
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    };
    let _ = stop.recv();
    // Whatever happens in the stop (a panic included), the process ends as the request would have ended it.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| node.stop_keeping_agents()));
    agora_platform::end_as_stopped()
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

/// `agora agent list`: one line per agent, columns separated by tabs (id, kind, state, running or stopped), and, after a node restart,
/// the agents it found interrupted (ADR-0107) with a fifth column; see [`agent_list_lines`].
fn agent_list(say: &Say, o: &Opts) -> ExitCode {
    let mut c = match connect(say, o) {
        Ok(c) => c,
        Err(code) => return code,
    };
    match call(say, &mut c, Cmd::AgentList(v1::AgentList::default())) {
        Ok(v1::CommandOk {
            result: Some(Ok_::AgentList(list)),
        }) => {
            let lines = agent_list_lines(&list);
            if lines.is_empty() {
                say.out(key!("cli-agent-list-empty"), &[]);
            }
            for line in lines {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Ok(_) => ExitCode::from(1),
        Err(code) => code,
    }
}

/// The lines of `agora agent list`, one list in id order (`agent-2` before `agent-10`) across the node's agents and the ones it found
/// interrupted. An agent's line is `id, kind, state, running|stopped`, as it always was. ADR-0107: an interrupted agent's line has its
/// last state, `interrupted` in the fourth column and what can be done in a fifth: `still-running` (its process outlived the node;
/// `agora agent stop` ends it, and Resume waits until then), `resume`, or `start-again` (no session to continue). Fixed tokens like
/// `running` and `stopped`, not prose, so scripts can read them.
fn agent_list_lines(list: &v1::AgentListResult) -> Vec<String> {
    fn state_word(s: i32) -> &'static str {
        v1::AgentState::try_from(s).map_or("unknown", |s| match s {
            v1::AgentState::Working => "working",
            v1::AgentState::Blocked => "blocked",
            v1::AgentState::Idle => "idle",
            v1::AgentState::Done => "done",
            v1::AgentState::Unspecified => "unknown",
        })
    }
    let mut rows: Vec<(&str, String)> = list
        .agents
        .iter()
        .map(|a| {
            (
                a.agent_id.as_str(),
                format!(
                    "{}\t{}\t{}\t{}",
                    a.agent_id,
                    a.kind,
                    state_word(a.state),
                    if a.running { "running" } else { "stopped" }
                ),
            )
        })
        .chain(list.interrupted.iter().map(|i| {
            let next = if i.still_running {
                "still-running"
            } else if i.resumable {
                "resume"
            } else {
                "start-again"
            };
            (
                i.agent_id.as_str(),
                format!(
                    "{}\t{}\t{}\tinterrupted\t{next}",
                    i.agent_id,
                    i.kind,
                    state_word(i.last_state)
                ),
            )
        }))
        .collect();
    // Stable: rows with the same number keep the node's order.
    rows.sort_by_key(|(id, _)| {
        id.strip_prefix("agent-")
            .and_then(|n| n.parse::<u64>().ok())
            .unwrap_or(u64::MAX)
    });
    rows.into_iter().map(|(_, line)| line).collect()
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

/// `agora agent resume AGENT_ID [--again]` (#1137): brings back an agent `agora agent list` shows as interrupted, under the same id in
/// the same folder: with its own session (Resume, `resume` in the list), or with `--again` a new one (Start again, `start-again`). The
/// same `AgentResume` the window and the terminal client send (ADR-0060); unsigned, as `agora agent start` is, since the local socket is
/// this machine's own user (ADR-0082). The node answers why it cannot (`agent.not_interrupted`, `agent.session_missing`, ...).
fn agent_resume(say: &Say, o: &Opts) -> ExitCode {
    let Some(id) = o.positional.first() else {
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    };
    let mut c = match connect(say, o) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let cmd = Cmd::AgentResume(v1::AgentResume {
        agent_id: id.clone(),
        new_session: o.again,
        authorization: None,
    });
    match call(say, &mut c, cmd) {
        Ok(v1::CommandOk {
            result: Some(Ok_::AgentLaunch(r)),
        }) => {
            say.out(
                if o.again {
                    key!("cli-agent-started-again")
                } else {
                    key!("cli-agent-resumed")
                },
                &[("agent_id", r.agent_id.into()), ("worktree", r.path.into())],
            );
            ExitCode::SUCCESS
        }
        Ok(_) => ExitCode::from(1),
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
            history_lines: 0,
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
    let _ = stdout.write_all(ATTACH_START).and_then(|()| stdout.flush());
    let mut keyboard = HostKeyboard::default();
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
                KIND_TERM_SNAPSHOT => {
                    let repaint = repaint_for_host(&f.payload);
                    keyboard.feed(&repaint);
                    let _ = stdout.write_all(&repaint).and_then(|()| stdout.flush());
                }
                KIND_TERM_DATA => {
                    keyboard.feed(&f.payload);
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
    let _ = stdout
        .write_all(&keyboard.detach())
        .and_then(|()| stdout.write_all(DETACH_RESET))
        .and_then(|()| stdout.flush());
    drop(raw);
    println!();
    say.out(
        key!("cli-terminal-detached"),
        &[("agent_id", agent.as_str().into())],
    );
    ExitCode::SUCCESS
}

#[cfg(test)]
mod host_keyboard_tests {
    use super::HostKeyboard;

    fn pops(stream: &[u8], split: usize) -> String {
        let mut k = HostKeyboard::default();
        for c in stream.chunks(split) {
            k.feed(c);
        }
        String::from_utf8_lossy(&k.detach()).into_owned()
    }

    #[test]
    fn detach_pops_our_entry_and_what_the_agent_left_on_the_main_screen() {
        for split in [1, 3, 1 << 16] {
            for (stream, want, what) in [
                (&b""[..], "\x1b[<1u", "our entry"),
                (b"\x1b[>5u\x1b[>7u", "\x1b[<3u", "and two of the agent's"),
                (b"\x1b[>5u\x1b[>7u\x1b[<u", "\x1b[<2u", "one it popped"),
                // The agent popped ours and the terminal's below it: nothing of ours is left to pop.
                (b"\x1b[>5u\x1b[<9u", "", "past ours"),
                (
                    b"\x1b[>5u\x1b[<9u\x1b[>3u",
                    "\x1b[<1u",
                    "what it pushed after that",
                ),
                (b"\x1bc", "", "a full reset"),
                // On the alternate screen the stack is the screen's own and goes with it; detach leaves that screen first.
                (
                    b"\x1b[>1u\x1b[?1049h\x1b[>5u\x1b[>7u",
                    "\x1b[?1049l\x1b[<2u",
                    "alternate screen",
                ),
                (
                    b"\x1b[?1049h\x1b[>5u\x1b[?1049l\x1b[>3u",
                    "\x1b[<2u",
                    "back on the main screen",
                ),
                (
                    b"\x1b[?1;1049h\x1b[>5u",
                    "\x1b[?1049l\x1b[<1u",
                    "1049 in a list",
                ),
                // Other sequences ending in u or h change nothing: restore cursor, set flags, an ordinary mode.
                (b"\x1b[u\x1b[=5;1u\x1b[?2004h\x1b[>5u", "\x1b[<2u", "others"),
            ] {
                assert_eq!(pops(stream, split), want, "{what}, split {split}");
            }
        }
    }
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

    /// Codex re-review of F1031: a node that ends because its link folder cannot be read yet (a volume still being mounted) must not
    /// end so soon that its supervisor takes it for a refusal and stops trying.
    #[test]
    fn a_node_waits_for_an_unreadable_link_folder_longer_than_a_refusal() {
        let refusal = agora_core::restart::RestartRule::default().refusal_ms;
        assert!(LINK_UNREADABLE_RETRY.as_millis() > u128::from(refusal));
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

    #[test]
    fn an_error_whose_last_parameter_is_empty_or_padded_ends_at_its_text() {
        // `adapter-agent_exited` ends in the agent's last output, which can be empty or end in a line break.
        for (tail, end) in [("", "output:"), ("boom\n", "output: boom")] {
            let e = Error::new(ErrorCode::AdapterAgentExited)
                .with("stage", "turn")
                .with("exit_code", "1")
                .with("stderr_tail", tail)
                .to_info();
            let en = Say(Catalog::english()).remote_error_text(&e);
            assert!(en.ends_with(end), "{en:?}");
        }
    }
}

#[cfg(test)]
mod agent_list_tests {
    use super::*;

    fn summary(id: &str, state: v1::AgentState, running: bool) -> v1::AgentSummary {
        v1::AgentSummary {
            agent_id: id.into(),
            kind: "shell".into(),
            state: state as i32,
            worktree_id: "/w".into(),
            terminal_id: String::new(),
            running,
            restarts: 0,
        }
    }

    fn interrupted(id: &str, resumable: bool, still_running: bool) -> v1::AgentInterrupted {
        v1::AgentInterrupted {
            agent_id: id.into(),
            kind: "claude-code".into(),
            project_id: "p".into(),
            folder: "/w".into(),
            parent_agent_id: String::new(),
            resumable,
            started_at_ms: 1,
            interrupted_at_ms: 2,
            last_state: v1::AgentState::Working as i32,
            still_running,
        }
    }

    #[test]
    fn interrupted_agents_get_a_fifth_column_and_one_id_order_across_both_lists() {
        let list = v1::AgentListResult {
            agents: vec![
                summary("agent-2", v1::AgentState::Done, false),
                summary("agent-10", v1::AgentState::Idle, true),
            ],
            interrupted: vec![
                interrupted("agent-3", true, false),
                interrupted("agent-4", false, false),
                // A survivor can be resumable too: Resume waits until it is stopped, so it says still-running.
                interrupted("agent-11", true, true),
            ],
        };
        assert_eq!(
            agent_list_lines(&list),
            [
                "agent-2\tshell\tdone\tstopped",
                "agent-3\tclaude-code\tworking\tinterrupted\tresume",
                "agent-4\tclaude-code\tworking\tinterrupted\tstart-again",
                "agent-10\tshell\tidle\trunning",
                "agent-11\tclaude-code\tworking\tinterrupted\tstill-running",
            ]
        );
    }

    /// An old node sends no interrupted agents: the lines are exactly the ones `agora agent list` printed before ADR-0107.
    #[test]
    fn without_interrupted_agents_the_lines_are_unchanged() {
        let list = v1::AgentListResult {
            agents: vec![
                summary("agent-1", v1::AgentState::Working, true),
                summary("agent-2", v1::AgentState::Unspecified, false),
            ],
            interrupted: Vec::new(),
        };
        assert_eq!(
            agent_list_lines(&list),
            [
                "agent-1\tshell\tworking\trunning",
                "agent-2\tshell\tunknown\tstopped"
            ]
        );
        assert!(agent_list_lines(&v1::AgentListResult::default()).is_empty());
    }

    /// Only interrupted agents: listed, and the "no agents" sentence is not said (the caller says it for no lines at all).
    #[test]
    fn a_node_with_only_interrupted_agents_has_lines() {
        let list = v1::AgentListResult {
            agents: Vec::new(),
            interrupted: vec![interrupted("agent-1", true, false)],
        };
        assert_eq!(
            agent_list_lines(&list),
            ["agent-1\tclaude-code\tworking\tinterrupted\tresume"]
        );
    }
}
