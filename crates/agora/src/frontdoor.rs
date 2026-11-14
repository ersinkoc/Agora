//! `agora` with no arguments: the front door (ADR-0062, rung 1 of `docs/map.md`).
//!
//! The first time, a short choice writes `<AGORA_HOME>/machine.toml` (what this machine runs; never a secret). Every time after, the
//! roles it remembers (a hub, then a node enrolled with that hub) are started **detached** when they are not running, and the
//! interface opens signed in: the terminal client by default, the browser with `agora web` or `--ui web`. Closing or killing the
//! interface changes nothing for the hub, the node or the agents: they are processes of their own. `agora status` shows them and
//! `agora down` stops what the front door started.

use std::io::{BufRead, IsTerminal, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use agora_i18n::{key, Arg};
use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};
use agora_tui::client::{sign_in, Hub, Session, Update};

use crate::hubcli::{enroll_and_save, new_enrollment_code, new_login_code, EnrollFailure};
use crate::{home, Opts, Say};

pub(crate) const MACHINE_FILE: &str = "machine.toml";
const DEFAULT_PORT: u16 = 7443;
/// How long a started hub or node has to answer before the front door gives up and points at its log.
const WAIT: Duration = Duration::from_secs(20);
/// A log larger than this at start is renamed to `<name>.1` (replacing the previous one) and a new one begun.
const LOG_LIMIT: u64 = 5 * 1024 * 1024;

/// The interface `agora` opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ui {
    Tui,
    Web,
    None,
}

impl Ui {
    /// What a machine without a saved choice opens: the browser window on a desktop, the terminal client when there is no screen
    /// to open a browser on (an SSH session, or Linux with no display).
    pub(crate) fn default_for(env: &dyn Fn(&str) -> bool) -> Ui {
        let ssh = env("SSH_CONNECTION") || env("SSH_TTY");
        let headless_linux =
            std::env::consts::OS == "linux" && !env("DISPLAY") && !env("WAYLAND_DISPLAY");
        if ssh || headless_linux {
            Ui::Tui
        } else {
            Ui::Web
        }
    }

    fn system_default() -> Ui {
        Ui::default_for(&|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
    }

    fn parse(s: &str) -> Option<Ui> {
        match s {
            "tui" => Some(Ui::Tui),
            "web" => Some(Ui::Web),
            "none" => Some(Ui::None),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Ui::Tui => "tui",
            Ui::Web => "web",
            Ui::None => "none",
        }
    }
}

/// `<AGORA_HOME>/machine.toml`: what this machine runs. Owner-only; never holds a secret (codes, tokens and keys stay in the hub's
/// folder and the link folder).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Machine {
    /// `hub`, `node`.
    pub roles: Vec<String>,
    pub ui: Ui,
    /// The address the hub listens on (`127.0.0.1:7443`, or `0.0.0.0:PORT` when other machines may connect).
    pub hub_listen: String,
    /// The names in the hub's certificate.
    pub hub_names: Vec<String>,
    pub hub_dir: PathBuf,
    /// The folders the node offers as projects.
    pub node_projects: Vec<PathBuf>,
}

impl Machine {
    pub(crate) fn has(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }

    /// The hub's port.
    fn port(&self) -> Option<u16> {
        self.hub_listen.rsplit_once(':')?.1.parse().ok()
    }

    /// Where this machine reaches its own hub (loopback, whatever address the hub listens on).
    fn local_addr(&self) -> Option<String> {
        Some(format!("127.0.0.1:{}", self.port()?))
    }

    /// The hub address a node enrolled with this machine's own hub keeps in its link folder.
    fn own_hub_url(&self) -> Option<String> {
        Some(format!("wss://localhost:{}", self.port()?))
    }

    pub(crate) fn to_toml(&self) -> String {
        let list = |v: &[String]| {
            let items: Vec<String> = v.iter().map(|s| quote(s)).collect();
            format!("[{}]", items.join(", "))
        };
        let paths: Vec<String> = self
            .node_projects
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        format!(
            "# What this machine runs (ADR-0062). Written by `agora`; it never holds a secret.\n\
             roles = {}\n\
             ui = {}\n\
             hub_listen = {}\n\
             hub_names = {}\n\
             hub_dir = {}\n\
             node_projects = {}\n",
            list(&self.roles),
            quote(self.ui.as_str()),
            quote(&self.hub_listen),
            list(&self.hub_names),
            quote(&self.hub_dir.to_string_lossy()),
            list(&paths),
        )
    }

    /// Reads the file `to_toml` writes (a small subset of TOML: one `key = "string"` or `key = ["string", ...]` per line).
    pub(crate) fn parse(text: &str) -> Option<Machine> {
        let mut roles = None;
        let mut ui = Ui::system_default();
        let mut hub_listen = None;
        let mut hub_names = Vec::new();
        let mut hub_dir = None;
        let mut node_projects = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (k, v) = line.split_once('=')?;
            let v = v.trim();
            match k.trim() {
                "roles" => roles = Some(parse_list(v)?),
                "ui" => ui = Ui::parse(&parse_string(v)?.0)?,
                "hub_listen" => hub_listen = Some(parse_string(v)?.0),
                "hub_names" => hub_names = parse_list(v)?,
                "hub_dir" => hub_dir = Some(PathBuf::from(parse_string(v)?.0)),
                "node_projects" => {
                    node_projects = parse_list(v)?.into_iter().map(PathBuf::from).collect()
                }
                // Keys of a later version are kept by that version; this one ignores them.
                _ => {}
            }
        }
        let roles: Vec<String> = roles?;
        let m = Machine {
            hub_listen: hub_listen.unwrap_or_default(),
            hub_dir: hub_dir.unwrap_or_default(),
            roles,
            ui,
            hub_names,
            node_projects,
        };
        if m.has("hub") && (m.port().is_none() || m.hub_dir.as_os_str().is_empty()) {
            return None;
        }
        Some(m)
    }
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\u{:04X}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A basic TOML string at the start of `s`: its value and what follows it.
fn parse_string(s: &str) -> Option<(String, &str)> {
    let rest = s.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, &rest[i + 1..])),
            '\\' => match chars.next()?.1 {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'u' => {
                    let hex: String = (0..4).filter_map(|_| chars.next().map(|x| x.1)).collect();
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                }
                _ => return None,
            },
            c => out.push(c),
        }
    }
    None
}

fn parse_list(s: &str) -> Option<Vec<String>> {
    let mut rest = s.strip_prefix('[')?.trim_start();
    let mut out = Vec::new();
    loop {
        if let Some(r) = rest.strip_prefix(']') {
            return r.trim().is_empty().then_some(out);
        }
        let (v, r) = parse_string(rest)?;
        out.push(v);
        rest = r.trim_start();
        if let Some(r) = rest.strip_prefix(',') {
            rest = r.trim_start();
        }
    }
}

fn machine_path(home: &Path) -> PathBuf {
    home.join(MACHINE_FILE)
}

pub(crate) fn load(home: &Path) -> Option<Machine> {
    Machine::parse(&std::fs::read_to_string(machine_path(home)).ok()?)
}

fn save(home: &Path, m: &Machine) -> std::io::Result<()> {
    std::fs::create_dir_all(home)?;
    agora_platform::write_private(&machine_path(home), m.to_toml().as_bytes())
}

/// The host name up to its first dot: `mac.lan` is `mac`.
fn short_name(hostname: &str) -> String {
    let h = hostname.trim();
    h.split('.').next().unwrap_or(h).to_string()
}

/// The machine's short name (`mac` for `mac.lan`) as a node id: lower case letters, digits, `_` and `-` (anything else becomes
/// `-`), at most 64 characters.
fn node_id_from(hostname: Option<String>) -> String {
    let id: String = short_name(&hostname.unwrap_or_default())
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    let id = id.trim_matches(['-', '.']).to_string();
    if id.is_empty() {
        "this-machine".into()
    } else {
        id
    }
}

/// This machine's IPv4 addresses on its networks, found without sending anything: a UDP socket "connected" towards an address only
/// picks the local address the system would route from.
fn lan_ipv4() -> Vec<Ipv4Addr> {
    let mut out = Vec::new();
    for probe in [
        "192.168.255.255:9",
        "10.255.255.255:9",
        "172.31.255.255:9",
        "8.8.8.8:9",
    ] {
        let found = UdpSocket::bind("0.0.0.0:0")
            .and_then(|s| s.connect(probe).map(|()| s))
            .and_then(|s| s.local_addr());
        if let Ok(SocketAddr::V4(a)) = found {
            let ip = *a.ip();
            if !ip.is_loopback() && !ip.is_unspecified() && !out.contains(&ip) {
                out.push(ip);
            }
        }
    }
    out
}

/// Whether something accepts connections on this machine's `port`.
fn answers(port: u16) -> bool {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok()
}

/// The first port from `from` that `ip` can listen on and nobody answers on.
fn free_port(ip: IpAddr, from: u16) -> Option<u16> {
    (from..from.saturating_add(100)).find(|p| {
        if answers(*p) {
            return false;
        }
        // One at a time: each test listener is closed before the next is tried.
        let wanted = TcpListener::bind(SocketAddr::new(ip, *p)).is_ok();
        let loopback =
            TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), *p)).is_ok();
        wanted && loopback
    })
}

/// The first-run choice. `None` when the person chose to join (not built yet) or input ended.
enum Choice {
    Here,
    Alone,
    Join,
}

fn prompt(out: &mut dyn Write, text: &str) {
    // The question and the answer on one line.
    let _ = write!(out, "{text} ");
    let _ = out.flush();
}

fn read_answer(input: &mut dyn BufRead) -> Option<String> {
    let mut line = String::new();
    match input.read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim().to_lowercase()),
    }
}

/// Asks the first-run questions and builds the machine's settings (`None`: the person chose to join, or input ended).
fn ask(
    say: &Say,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    home: &Path,
    cwd: &Path,
    ui: Ui,
) -> Result<Option<Machine>, ExitCode> {
    let _ = writeln!(out, "{}", say.text(key!("frontdoor-welcome"), &[]));
    let _ = writeln!(out, "{}", say.text(key!("frontdoor-choice-here"), &[]));
    let _ = writeln!(out, "{}", say.text(key!("frontdoor-choice-alone"), &[]));
    let _ = writeln!(out, "{}", say.text(key!("frontdoor-choice-join"), &[]));
    let choice = loop {
        prompt(out, &say.text(key!("frontdoor-choose"), &[]));
        match read_answer(input).as_deref() {
            None => return Err(ExitCode::from(2)),
            Some("1") => break Choice::Here,
            Some("2") => break Choice::Alone,
            Some("3") => break Choice::Join,
            Some(_) => {}
        }
    };
    let lan = match choice {
        Choice::Join => {
            let _ = writeln!(out, "{}", say.text(key!("frontdoor-join-later"), &[]));
            return Ok(None);
        }
        Choice::Alone => false,
        Choice::Here => {
            prompt(out, &say.text(key!("frontdoor-lan-question"), &[]));
            matches!(
                read_answer(input).as_deref(),
                Some("y" | "yes" | "e" | "evet")
            )
        }
    };
    let ip = if lan {
        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
    } else {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    };
    let Some(port) = free_port(ip, DEFAULT_PORT) else {
        say.err(
            key!("frontdoor-no-port"),
            &[("port", i64::from(DEFAULT_PORT).into())],
        );
        return Err(ExitCode::from(1));
    };
    let mut names = vec!["localhost".to_string()];
    if lan {
        if let Some(full) = agora_platform::hostname() {
            // The short name (`mac`) and, as a certificate name only, the full one (`mac.lan`).
            for h in [short_name(&full), full] {
                let h: String = h
                    .trim()
                    .to_lowercase()
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.'))
                    .collect();
                if !h.is_empty() && !names.contains(&h) {
                    names.push(h);
                }
            }
        }
        names.extend(lan_ipv4().iter().map(Ipv4Addr::to_string));
    }
    // Only a git checkout is offered as a project right away; any other folder is chosen later in the interface.
    let projects = if cwd.join(".git").exists() {
        vec![cwd.to_path_buf()]
    } else {
        Vec::new()
    };
    Ok(Some(Machine {
        roles: vec!["hub".into(), "node".into()],
        ui,
        hub_listen: SocketAddr::new(ip, port).to_string(),
        hub_names: names,
        hub_dir: home.join("hub"),
        node_projects: projects,
    }))
}

/// `agora [--ui tui|web|none [--save]]`: `--ui` alone applies to this run; with `--save` it becomes the default.
pub fn front_door(say: &Say, o: &Opts) -> ExitCode {
    let ui = match o.ui.as_deref().map(Ui::parse) {
        None => None,
        Some(Some(u)) => Some(u),
        Some(None) => {
            say.err(key!("cli-bad-arguments"), &[]);
            return ExitCode::from(2);
        }
    };
    let h = home();
    let mut first_run = false;
    let m = match load(&h) {
        Some(mut m) => {
            if let Some(u) = ui.filter(|u| o.save && *u != m.ui) {
                m.ui = u;
                if let Err(e) = save(&h, &m) {
                    return failed(say, &e.to_string());
                }
            }
            m
        }
        None if machine_path(&h).exists() => {
            say.err(
                key!("frontdoor-bad-machine-file"),
                &[("path", machine_path(&h).display().to_string().into())],
            );
            return ExitCode::from(1);
        }
        None => {
            if !std::io::stdin().is_terminal() {
                say.err(
                    key!("frontdoor-get-started"),
                    &[("path", machine_path(&h).display().to_string().into())],
                );
                return ExitCode::from(2);
            }
            // A hub folder or a link made by hand (`agora hub run`, `agora node enroll`) may belong to a hub that runs now on a port
            // this file does not know: starting a second hub on the same database, or enrolling again, would break it.
            let by_hand = [h.join("hub").join("hub.db"), h.join("link").join("node-id")];
            if let Some(found) = by_hand.iter().find(|p| p.exists()) {
                say.err(
                    key!("frontdoor-set-up-by-hand"),
                    &[("path", found.display().to_string().into())],
                );
                return ExitCode::from(1);
            }
            let cwd = std::env::current_dir().unwrap_or_default();
            let asked = ask(
                say,
                &mut std::io::stdin().lock(),
                &mut std::io::stdout(),
                &h,
                &cwd,
                ui.filter(|_| o.save).unwrap_or_else(Ui::system_default),
            );
            match asked {
                Ok(Some(m)) => {
                    if let Err(e) = save(&h, &m) {
                        return failed(say, &e.to_string());
                    }
                    say.out(
                        key!("frontdoor-saved"),
                        &[("path", machine_path(&h).display().to_string().into())],
                    );
                    first_run = true;
                    m
                }
                Ok(None) => return ExitCode::SUCCESS,
                Err(code) => return code,
            }
        }
    };
    if let Err(code) = up(say, &h, &m) {
        return code;
    }
    // ADR-0062 item 6: the end of the first run asks once whether to start Agora at login. Always a question, never automatic; a no
    // (or no answer) leaves it to `agora service install`.
    // Only where this machine can register one (Linux: systemd's `systemctl`, found at run time).
    if first_run && agora_platform::services_available() {
        prompt(
            &mut std::io::stdout(),
            &say.text(key!("frontdoor-autostart-question"), &[]),
        );
        if wants_autostart(read_answer(&mut std::io::stdin().lock()).as_deref()) {
            // Registered for the roles that run now; the output says so (they run already, so nothing starts twice).
            let _ = crate::servicecmd::install(say, &h, &m);
        } else {
            say.out(key!("frontdoor-autostart-later"), &[]);
        }
    }
    open(say, &m, ui.unwrap_or(m.ui))
}

/// The answer to "Start Agora when you log in? [Y/n]": Enter or yes is yes, no is no, and no answer at all (the input ended) is no.
fn wants_autostart(answer: Option<&str>) -> bool {
    matches!(answer, Some("" | "y" | "yes" | "e" | "evet"))
}

/// `agora web`: makes sure this machine's Agora runs, then opens it in the browser, signed in.
pub fn web(say: &Say) -> ExitCode {
    let h = home();
    let Some(m) = load(&h) else {
        say.err(key!("frontdoor-not-set-up"), &[]);
        return ExitCode::from(1);
    };
    if let Err(code) = up(say, &h, &m) {
        return code;
    }
    open(say, &m, Ui::Web)
}

/// A failure to take or change a role's lock, said in words: a guard held too long by another process (stopped or hung) is named with
/// its file, anything else is passed on as the reason.
pub(crate) fn lock_failed(say: &Say, lock: &Path, e: &std::io::Error) -> ExitCode {
    let mut guard = lock.as_os_str().to_owned();
    guard.push(".guard");
    let path: Arg = PathBuf::from(guard).display().to_string().into();
    // The refusals of `agora_platform::open_lock_private` / `ensure_private_dir`, named for what they are.
    let id = match e.kind() {
        std::io::ErrorKind::TimedOut => key!("frontdoor-lock-busy"),
        std::io::ErrorKind::InvalidInput => key!("frontdoor-lock-unsafe"),
        std::io::ErrorKind::PermissionDenied => key!("frontdoor-lock-not-yours"),
        _ => return failed(say, &e.to_string()),
    };
    say.err(id, &[("path", path)]);
    ExitCode::from(1)
}

/// [`lock_failed`] after `agora down` stopped (or found ended) the lock's process: a busy guard leaves only its old lock file behind,
/// which the next start replaces, and is said that way.
fn down_lock_failed(say: &Say, lock: &Path, e: &std::io::Error) -> ExitCode {
    if e.kind() != std::io::ErrorKind::TimedOut {
        return lock_failed(say, lock, e);
    }
    let mut guard = lock.as_os_str().to_owned();
    guard.push(".guard");
    say.err(
        key!("down-lock-busy"),
        &[("path", PathBuf::from(guard).display().to_string().into())],
    );
    ExitCode::from(1)
}

fn failed(say: &Say, reason: &str) -> ExitCode {
    say.err(key!("frontdoor-failed"), &[("reason", reason.into())]);
    ExitCode::from(1)
}

fn open(say: &Say, m: &Machine, ui: Ui) -> ExitCode {
    let (Some(addr), true) = (m.local_addr(), m.has("hub")) else {
        // A machine without a hub of its own (joining comes in a later rung) has nothing to open here yet.
        say.out(key!("frontdoor-running"), &[]);
        return ExitCode::SUCCESS;
    };
    match ui {
        Ui::None => {
            say.out(key!("frontdoor-running"), &[]);
            ExitCode::SUCCESS
        }
        Ui::Tui => crate::tuicmd::tui(
            say,
            &Opts {
                hub: Some(addr),
                hub_dir: Some(m.hub_dir.clone()),
                ..Opts::default()
            },
        ),
        Ui::Web => {
            let code = match new_login_code(&m.hub_dir) {
                Ok(c) => c,
                Err(e) => {
                    say.err(key!("frontdoor-web-failed"), &[("reason", e.into())]);
                    return ExitCode::from(1);
                }
            };
            let Some(port) = m.port() else {
                return failed(say, &m.hub_listen);
            };
            // The code is in the fragment, which the browser never sends; the page trades it for a session at once.
            let url = format!("https://localhost:{port}/#code={code}");
            if agora_platform::open_url(&url).is_ok() {
                say.out(key!("frontdoor-web-opened"), &[]);
            } else {
                say.out(key!("frontdoor-web-url"), &[("url", url.into())]);
            }
            ExitCode::SUCCESS
        }
    }
}

// ----- the roles and their locks -----

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Role {
    Hub,
    Node,
}

impl Role {
    pub(crate) fn from_name(name: &str) -> Option<Role> {
        match name {
            "hub" => Some(Role::Hub),
            "node" => Some(Role::Node),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Role::Hub => "hub",
            Role::Node => "node",
        }
    }

    pub(crate) fn lock(self, home: &Path) -> PathBuf {
        home.join("run").join(format!("{}.lock", self.name()))
    }

    pub(crate) fn log(self, home: &Path) -> PathBuf {
        home.join("logs").join(format!("{}.log", self.name()))
    }

    /// The sentence naming this role in messages.
    pub(crate) fn label(self, say: &Say) -> String {
        say.text(
            match self {
                Role::Hub => key!("frontdoor-role-hub"),
                Role::Node => key!("frontdoor-role-node"),
            },
            &[],
        )
    }
}

/// A lock file: the process holding a role, when it started (so a reused pid is not mistaken for it) and its version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LockInfo {
    pub(crate) pid: u32,
    pub(crate) start: u64,
    version: String,
}

impl LockInfo {
    pub(crate) fn of(pid: u32) -> Option<LockInfo> {
        Some(LockInfo {
            pid,
            start: agora_platform::process_start_time(pid)?,
            version: env!("CARGO_PKG_VERSION").to_string(),
        })
    }

    pub(crate) fn text(&self) -> String {
        format!("{} {} {}\n", self.pid, self.start, self.version)
    }

    fn parse(text: &str) -> Option<LockInfo> {
        let mut w = text.split_whitespace();
        Some(LockInfo {
            pid: w.next()?.parse().ok()?,
            start: w.next()?.parse().ok()?,
            version: w.next().unwrap_or("").to_string(),
        })
    }

    pub(crate) fn alive(&self) -> bool {
        agora_platform::process_start_time(self.pid) == Some(self.start)
    }
}

pub(crate) fn read_lock(path: &Path) -> Option<LockInfo> {
    LockInfo::parse(&std::fs::read_to_string(path).ok()?)
}

/// The live holder of a role's lock, if any.
pub(crate) fn holder(home: &Path, role: Role) -> Option<LockInfo> {
    read_lock(&role.lock(home)).filter(LockInfo::alive)
}

pub(crate) enum Claim {
    /// This process holds the lock now and must start the role.
    Mine,
    /// A live process holds it (a running role, or another `agora` starting it).
    Held,
}

/// Holds `<lock>.guard` exclusively (an OS file lock, released when the returned file is dropped or the process ends). Every change
/// of a role's lock happens under it: taking or replacing it ([`claim`], the front door's handover), and removing it
/// ([`remove_lock_of`]). So a check of what the lock names and the change that follows are one step for every other process: `agora
/// down` can never delete a lock that a new supervisor put in place between its check and its removal (Codex's review of #548).
///
/// The guard and its `run` folder are private (`agora_platform::open_lock_private`, `ensure_private_dir`): another local user can
/// neither hold the guard nor make it land on another file. Waiting for it is bounded ([`GUARD_WAIT`]): a holder that is stopped (or
/// hangs) makes this fail with `TimedOut`, never wait forever.
fn guard(lock: &Path) -> std::io::Result<std::fs::File> {
    guard_within(lock, GUARD_WAIT)
}

/// [`guard`], waiting at most `wait`.
fn guard_within(lock: &Path, wait: Duration) -> std::io::Result<std::fs::File> {
    if let Some(dir) = lock.parent().filter(|d| !d.as_os_str().is_empty()) {
        agora_platform::ensure_private_dir(dir)?;
    }
    let mut path = lock.as_os_str().to_owned();
    path.push(".guard");
    let file = agora_platform::open_lock_private(Path::new(&path))?;
    let end = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < end => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(std::io::Error::from(std::io::ErrorKind::TimedOut))
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(e),
        }
    }
}

/// How long [`guard`] waits for another process to finish changing a role's lock. The longest a holder keeps it is `claim`'s 100 ms
/// pause, so this is only reached when the holder is stopped or hangs.
const GUARD_WAIT: Duration = Duration::from_secs(10);

/// Takes the role's lock for this process, replacing a stale one (its process ended, or its pid now belongs to another process).
pub(crate) fn claim(path: &Path) -> std::io::Result<Claim> {
    let me = LockInfo::of(std::process::id())
        .ok_or_else(|| std::io::Error::other("own start time unknown"))?;
    let _guard = guard(path)?;
    for _ in 0..3 {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        match options.open(path) {
            Ok(mut f) => {
                f.write_all(me.text().as_bytes())?;
                return Ok(Claim::Mine);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if read_lock(path).is_some_and(|l| l.alive()) {
                    return Ok(Claim::Held);
                }
                // A lock just created and not written yet reads as empty: give its writer a moment before calling it stale.
                std::thread::sleep(Duration::from_millis(100));
                if read_lock(path).is_some_and(|l| l.alive()) {
                    return Ok(Claim::Held);
                }
                let _ = std::fs::remove_file(path);
            }
            Err(e) => return Err(e),
        }
    }
    Ok(Claim::Held)
}

/// Renames a log over 5 MiB to `<log>.1` before a new start.
///
/// Only a plain file in a real folder is renamed: a `logs` folder that is a link (or a log that is one) is left as it is, and the open
/// that follows refuses it (#556), so a rename never lands in another folder.
pub(crate) fn rotate(log: &Path) {
    let real_folder = log.parent().is_none_or(|dir| {
        std::fs::symlink_metadata(dir)
            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink() && !is_reparse(&m))
    });
    let plain_big_file = std::fs::symlink_metadata(log)
        .is_ok_and(|m| m.is_file() && !is_reparse(&m) && m.len() > LOG_LIMIT);
    if real_folder && plain_big_file {
        let mut old = log.as_os_str().to_owned();
        old.push(".1");
        let _ = std::fs::rename(log, PathBuf::from(old));
    }
}

/// A Windows junction or other reparse point (which `is_symlink` does not always name); never true elsewhere.
fn is_reparse(m: &std::fs::Metadata) -> bool {
    agora_platform::is_reparse_point(m)
}

/// Whether the role answers: the hub on its port, the node on its control socket.
pub(crate) fn role_answers(home: &Path, m: &Machine, role: Role) -> bool {
    match role {
        Role::Hub => m.port().is_some_and(answers),
        Role::Node => agora_node::control::Client::connect(&home.join("node.sock")).is_ok(),
    }
}

/// The command line that runs `role` (what its supervisor runs).
pub(crate) fn command(
    home: &Path,
    m: &Machine,
    role: Role,
) -> std::io::Result<std::process::Command> {
    let mut cmd = std::process::Command::new(std::env::current_exe()?);
    cmd.env("AGORA_HOME", home).current_dir(home);
    match role {
        Role::Hub => {
            cmd.args(["hub", "run", "--listen", &m.hub_listen, "--hub-dir"])
                .arg(&m.hub_dir);
            for n in &m.hub_names {
                cmd.args(["--name", n]);
            }
        }
        Role::Node => {
            cmd.args(["node", "run"]);
            for p in &m.node_projects {
                cmd.arg("--project").arg(p);
            }
        }
    }
    Ok(cmd)
}

/// The command line of `role`'s supervisor (`agora supervise <role>`), which runs [`command`] and starts it again after a crash.
/// The supervisor waits only for this process (`--handoff PID:START`, the holder of the lock now) to hand the lock over; any other
/// holder makes it refuse at once, so a supervisor never takes a lock that `agora down` just freed.
fn supervisor_command(home: &Path, role: Role) -> std::io::Result<std::process::Command> {
    let me = LockInfo::of(std::process::id())
        .ok_or_else(|| std::io::Error::other("own start time unknown"))?;
    let mut cmd = std::process::Command::new(std::env::current_exe()?);
    cmd.env("AGORA_HOME", home)
        .current_dir(home)
        .args(["supervise", role.name(), "--handoff"])
        .arg(format!("{}:{}", me.pid, me.start));
    Ok(cmd)
}

/// Starts `role` detached under its supervisor unless it runs already, then waits for it to answer. The lock is this process's
/// until the supervisor's id replaces it; the supervisor waits for that before it starts the role.
fn ensure(say: &Say, home: &Path, m: &Machine, role: Role) -> Result<(), ExitCode> {
    let lock = role.lock(home);
    let log = role.log(home);
    if holder(home, role).is_none() && !role_answers(home, m, role) {
        let mine = claim(&lock).map_err(|e| lock_failed(say, &lock, &e))?;
        if let Claim::Mine = mine {
            say.out(
                key!("frontdoor-starting"),
                &[("role", role.label(say).into())],
            );
            rotate(&log);
            let started = supervisor_command(home, role)
                .and_then(|mut c| agora_platform::spawn_detached(&mut c, &log))
                .map(|d| {
                    if d.in_job {
                        // Windows: the terminal's job would not let it go, so closing that terminal ends it. Said here and in its log.
                        let text = say.text(
                            key!("frontdoor-in-job"),
                            &[("role", role.label(say).into())],
                        );
                        eprintln!("{text}");
                        let _ = agora_platform::open_append_private(&log)
                            .and_then(|mut f| writeln!(f, "{text}"));
                    }
                    d.pid
                })
                .and_then(|pid| {
                    LockInfo::of(pid).ok_or_else(|| std::io::Error::other("ended at once"))
                })
                .and_then(|l| {
                    let _guard = guard(&lock)?;
                    agora_platform::write_private(&lock, l.text().as_bytes())
                });
            if let Err(e) = started {
                // Only this process's own lock: a supervisor may hold it by now.
                if let Some(me) = LockInfo::of(std::process::id()) {
                    // A stale lock of its own is harmless (the next start takes it over), so a busy guard is not a second failure here.
                    let _ = remove_lock_of(&lock, &me);
                }
                // The log refused (a link, a second name, another user's file): said in words, with its path.
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::InvalidInput | std::io::ErrorKind::PermissionDenied
                ) {
                    say.err(
                        key!("supervise-log-link"),
                        &[("log", log.display().to_string().into())],
                    );
                    return Err(ExitCode::from(1));
                }
                say.err(
                    key!("frontdoor-not-ready"),
                    &[
                        ("role", role.label(say).into()),
                        ("seconds", (WAIT.as_secs() as i64).into()),
                        ("log", log.display().to_string().into()),
                    ],
                );
                return Err(failed(say, &e.to_string()));
            }
        }
    }
    let end = Instant::now() + WAIT;
    while !role_answers(home, m, role) {
        // A process that ended will not answer: say so now instead of at the end of the wait.
        let ended = read_lock(&lock).is_some_and(|l| !l.alive());
        if ended || Instant::now() > end {
            say.err(
                key!("frontdoor-not-ready"),
                &[
                    ("role", role.label(say).into()),
                    ("seconds", (WAIT.as_secs() as i64).into()),
                    ("log", log.display().to_string().into()),
                ],
            );
            return Err(ExitCode::from(1));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

/// Brings up what the machine remembers: the hub, the node's enrollment with it (the first time), the node, and waits until the node
/// shows up at the hub.
fn up(say: &Say, home: &Path, m: &Machine) -> Result<(), ExitCode> {
    for d in ["logs", "run"] {
        std::fs::create_dir_all(home.join(d)).map_err(|e| failed(say, &e.to_string()))?;
    }
    if m.has("hub") {
        ensure(say, home, m, Role::Hub)?;
    }
    if m.has("node") {
        if m.has("hub") && !crate::hubcli::link_dir().join("node-id").exists() {
            enroll_self(say, m)?;
        }
        ensure(say, home, m, Role::Node)?;
        if m.has("hub") {
            wait_linked(say, home, m)?;
        }
    }
    Ok(())
}

/// Enrolls this machine's node with this machine's own hub: a one-time code made here and used at once, never shown.
fn enroll_self(say: &Say, m: &Machine) -> Result<(), ExitCode> {
    let (Some(url), Ok(ca_pem)) = (
        m.own_hub_url(),
        std::fs::read_to_string(m.hub_dir.join("ca.pem")),
    ) else {
        return Err(failed(say, &m.hub_dir.join("ca.pem").display().to_string()));
    };
    let code = new_enrollment_code(&m.hub_dir).map_err(|e| failed(say, &e))?;
    let id = node_id_from(agora_platform::hostname());
    match enroll_and_save(&url, &ca_pem, &code, &id) {
        Ok(_) => Ok(()),
        Err(EnrollFailure::Refused(code)) | Err(EnrollFailure::Other(code)) => {
            Err(failed(say, &code))
        }
    }
}

/// Waits until this machine's own hub sees the node online (when the node is linked to it).
fn wait_linked(say: &Say, home: &Path, m: &Machine) -> Result<(), ExitCode> {
    let link = crate::hubcli::link_dir();
    let read = |n: &str| {
        std::fs::read_to_string(link.join(n))
            .ok()
            .map(|t| t.trim().to_string())
    };
    // A node linked to another hub is not waited for here.
    if read("url") != m.own_hub_url() {
        return Ok(());
    }
    let (Some(node_id), Some(addr), Ok(ca_pem)) = (
        read("node-id"),
        m.local_addr(),
        std::fs::read_to_string(m.hub_dir.join("ca.pem")),
    ) else {
        return Ok(());
    };
    let hub = Hub { addr, ca_pem };
    let end = Instant::now() + WAIT;
    let mut token: Option<String> = None;
    while Instant::now() < end {
        if token.is_none() {
            token = new_login_code(&m.hub_dir)
                .ok()
                .and_then(|c| sign_in(&hub, &c).ok());
        }
        if let Some(t) = &token {
            if let Ok(mut s) = Session::connect(&hub, t) {
                let until = Instant::now() + Duration::from_secs(2);
                while Instant::now() < until {
                    if let Update::Snapshot(snap) = s.next(Duration::from_millis(500)) {
                        if snap.nodes.iter().any(|n| n.node_id == node_id && n.online) {
                            return Ok(());
                        }
                        break;
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    say.err(
        key!("frontdoor-not-linked"),
        &[
            ("seconds", (WAIT.as_secs() as i64).into()),
            ("log", Role::Node.log(home).display().to_string().into()),
        ],
    );
    Err(ExitCode::from(1))
}

// ----- status and down -----

/// The number of agents running on the node, if it answers.
fn agents_running(home: &Path) -> Option<i64> {
    let mut c = agora_node::control::Client::connect(&home.join("node.sock")).ok()?;
    match c
        .call(Cmd::AgentList(v1::AgentList::default()))
        .ok()?
        .ok()?
    {
        v1::CommandOk {
            result: Some(Ok_::AgentList(list)),
        } => Some(list.agents.iter().filter(|a| a.running).count() as i64),
        _ => None,
    }
}

/// A line under a role in `agora status`, indented under it.
fn detail(say: &Say, id: &str, args: &[(&str, Arg)]) {
    let _ = writeln!(std::io::stdout(), "  {}", say.text(id, args));
}

/// `agora status`: one screen about what this machine runs.
pub fn status(say: &Say) -> ExitCode {
    let h = home();
    let Some(m) = load(&h) else {
        say.err(key!("frontdoor-not-set-up"), &[]);
        return ExitCode::from(1);
    };
    say.out(
        key!("status-machine"),
        &[
            ("path", machine_path(&h).display().to_string().into()),
            ("ui", m.ui.as_str().into()),
        ],
    );
    for role in [Role::Hub, Role::Node] {
        if !m.has(role.name()) {
            continue;
        }
        let log: Arg = role.log(&h).display().to_string().into();
        let label: Arg = role.label(say).into();
        let (held, answers) = (holder(&h, role), role_answers(&h, &m, role));
        let waiting = held
            .as_ref()
            .filter(|_| !answers)
            .and_then(|l| crate::supervise::waiting_restart(&h, role).map(|r| (l.pid, r)));
        match (held.clone(), answers) {
            // Its supervisor runs, the role does not: it crashed and starts again shortly.
            (Some(_), _) if waiting.is_some() => {
                if let Some((pid, r)) = &waiting {
                    let _ = writeln!(
                        std::io::stdout(),
                        "{}",
                        crate::supervise::restarting_line(say, role, *pid, r)
                    );
                }
            }
            (Some(l), _) => match supervised_child(l.pid) {
                Some(child) => say.out(
                    key!("status-running-supervised"),
                    &[
                        ("role", label),
                        ("pid", i64::from(child).into()),
                        ("supervisor", i64::from(l.pid).into()),
                        ("version", l.version.into()),
                    ],
                ),
                None => say.out(
                    key!("status-running"),
                    &[
                        ("role", label),
                        ("pid", i64::from(l.pid).into()),
                        ("version", l.version.into()),
                    ],
                ),
            },
            (None, true) => say.out(key!("status-running-other"), &[("role", label)]),
            (None, false) => say.out(key!("status-stopped"), &[("role", label)]),
        }
        match role {
            Role::Hub => detail(
                say,
                key!("status-hub"),
                &[
                    ("addr", m.hub_listen.clone().into()),
                    ("dir", m.hub_dir.display().to_string().into()),
                ],
            ),
            Role::Node => {
                let link = crate::hubcli::link_dir();
                let read = |n: &str| {
                    std::fs::read_to_string(link.join(n))
                        .ok()
                        .map(|t| t.trim().to_string())
                };
                match (read("url"), read("node-id")) {
                    (Some(url), Some(id)) => detail(
                        say,
                        key!("status-node-linked"),
                        &[("hub", url.into()), ("id", id.into())],
                    ),
                    _ => detail(say, key!("status-node-not-linked"), &[]),
                }
                if let Some(n) = agents_running(&h) {
                    detail(say, key!("status-agents"), &[("count", n.into())]);
                }
            }
        }
        for line in crate::supervise::status_lines(say, &h, role, held.is_some(), answers) {
            let _ = writeln!(std::io::stdout(), "  {line}");
        }
        detail(say, key!("status-log"), &[("log", log)]);
    }
    ExitCode::SUCCESS
}

/// The role process a supervisor runs (its `agora` child), when `pid` is a supervisor; `None` for a role started without one (by an
/// earlier version).
fn supervised_child(pid: u32) -> Option<u32> {
    agora_platform::process_children(pid)
        .ok()?
        .into_iter()
        .find(|c| {
            agora_platform::process_name(*c).is_ok_and(|n| n.to_lowercase().starts_with("agora"))
        })
}

/// Removes the role's lock only while it still names `l` (the same pid and start time): a supervisor that took the lock after `l`
/// ended keeps it. An unreadable lock (one being written) is left alone too.
fn remove_lock_of(lock: &Path, l: &LockInfo) -> std::io::Result<()> {
    let _guard = guard(lock)?;
    if read_lock(lock).is_some_and(|now| now.pid == l.pid && now.start == l.start) {
        let _ = std::fs::remove_file(lock);
    }
    Ok(())
}

/// How many holders of one role `agora down` stops in a row: a supervisor that took the lock while the one before was being stopped is
/// stopped too.
const DOWN_ROUNDS: usize = 3;

/// Stops whoever holds `role`'s lock (its supervisor and the role with it), up to [`DOWN_ROUNDS`] holders in a row, saying so. Returns
/// whether it stopped one; a failure is said and makes `code` 1.
pub(crate) fn stop_role(
    say: &Say,
    h: &Path,
    machine: Option<&Machine>,
    role: Role,
    code: &mut ExitCode,
) -> bool {
    let lock = role.lock(h);
    let mut stopped_any = false;
    for _ in 0..DOWN_ROUNDS {
        let Some(l) = read_lock(&lock) else {
            break;
        };
        if !l.alive() {
            if let Err(e) = remove_lock_of(&lock, &l) {
                *code = down_lock_failed(say, &lock, &e);
            }
            break;
        }
        let label: Arg = role.label(say).into();
        let supervised = supervised_child(l.pid).is_some();
        if let Err(e) = agora_platform::terminate_process(l.pid) {
            say.err(
                key!("down-failed"),
                &[("role", label), ("reason", e.to_string().into())],
            );
            *code = ExitCode::from(1);
            break;
        }
        let end = Instant::now() + Duration::from_secs(10);
        while l.alive() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(100));
        }
        if l.alive() {
            say.err(
                key!("down-failed"),
                &[
                    ("role", label),
                    ("reason", say.text(key!("down-still-running"), &[]).into()),
                ],
            );
            *code = ExitCode::from(1);
            break;
        }
        // The process ended; its lock could not be removed only when another process holds the guard (stopped or hung): said, and the
        // command fails, instead of waiting another round and calling it done.
        let removed = remove_lock_of(&lock, &l);
        // What the ended process had open (its socket, its port) can linger a moment; status reads them right after.
        if let Some(m) = machine {
            let end = Instant::now() + Duration::from_secs(5);
            while role_answers(h, m, role) && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        stopped_any = true;
        say.out(
            if supervised {
                key!("down-stopped-supervised")
            } else {
                key!("down-stopped")
            },
            &[("role", label), ("pid", i64::from(l.pid).into())],
        );
        if let Err(e) = removed {
            *code = down_lock_failed(say, &lock, &e);
            break;
        }
    }
    stopped_any
}

/// `agora down`: stops what the front door started, the node first, then the hub.
pub fn down(say: &Say) -> ExitCode {
    let h = home();
    let machine = load(&h);
    let mut code = ExitCode::SUCCESS;
    let mut stopped_any = false;
    for role in [Role::Node, Role::Hub] {
        stopped_any |= stop_role(say, &h, machine.as_ref(), role, &mut code);
    }
    if !stopped_any && code == ExitCode::SUCCESS {
        say.out(key!("down-nothing"), &[]);
    }
    code
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_i18n::Catalog;

    fn say() -> Say {
        Say(Catalog::for_locale("en"))
    }

    #[test]
    fn the_login_question_defaults_to_yes_but_never_answers_itself() {
        for yes in ["", "y", "yes", "e", "evet"] {
            assert!(wants_autostart(Some(yes)), "{yes:?}");
        }
        for no in ["n", "no", "h", "hayır", "later"] {
            assert!(!wants_autostart(Some(no)), "{no:?}");
        }
        // The input ended (no person answered): nothing is registered.
        assert!(!wants_autostart(None));
    }

    /// Linux's follow-up to #543: `agora down` deletes a lock only while it names the process it stopped. A lock another supervisor
    /// took in the meantime, or one still being written, stays.
    /// Codex's review of #548: the check of what the lock names and its removal are one step against a supervisor replacing it. Here
    /// the "supervisor" holds the guard while it replaces the lock; the removal that started meanwhile waits for it, then sees the new
    /// holder and keeps the lock. Without the guard it would have deleted the lock it checked a moment before.
    #[test]
    fn removing_a_lock_waits_for_a_replacement_in_progress() {
        let dir = tmp("guard");
        let lock = dir.join("node.lock");
        let me = LockInfo::of(std::process::id()).unwrap();
        let stopped = LockInfo {
            pid: me.pid,
            start: me.start + 1,
            version: String::new(),
        };
        std::fs::write(&lock, stopped.text()).unwrap();
        let held = guard(&lock).unwrap();
        let remover = {
            let (lock, stopped) = (lock.clone(), stopped.clone());
            std::thread::spawn(move || remove_lock_of(&lock, &stopped).unwrap())
        };
        std::thread::sleep(Duration::from_millis(300));
        assert!(lock.exists(), "the removal did not wait for the guard");
        // The replacement: a new holder writes its lock, then lets go.
        std::fs::write(&lock, me.text()).unwrap();
        drop(held);
        remover.join().unwrap();
        assert_eq!(
            read_lock(&lock).map(|l| (l.pid, l.start)),
            Some((me.pid, me.start))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Linux's review of #551: a holder of the guard that is stopped (or hangs) makes the next change wait a bounded time and fail
    /// with `TimedOut`, never forever.
    #[test]
    fn a_guard_held_by_another_waits_a_bounded_time() {
        let dir = tmp("guard-wait");
        let lock = dir.join("run").join("node.lock");
        let held = guard(&lock).unwrap();
        let started = Instant::now();
        let e = guard_within(&lock, Duration::from_millis(300)).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5));
        drop(held);
        assert!(guard_within(&lock, Duration::from_millis(300)).is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Linux's follow-up to #560: when another process holds the guard, removing the lock fails (TimedOut) instead of returning as if
    /// done, so `agora down` can say so and fail.
    #[test]
    fn removing_a_lock_under_a_held_guard_fails() {
        let dir = tmp("guard-held-remove");
        let lock = dir.join("run").join("node.lock");
        let held = guard(&lock).unwrap();
        let me = LockInfo::of(std::process::id()).unwrap();
        std::fs::write(&lock, me.text()).unwrap();
        // A short wait, through the same path `remove_lock_of` takes.
        let e = guard_within(&lock, Duration::from_millis(200)).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::TimedOut);
        drop(held);
        remove_lock_of(&lock, &me).unwrap();
        assert!(!lock.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn down_removes_only_the_lock_of_the_process_it_stopped() {
        let dir = tmp("lock-of");
        let lock = dir.join("node.lock");
        let me = LockInfo::of(std::process::id()).unwrap();
        let stopped = LockInfo {
            pid: me.pid,
            start: me.start + 1,
            version: String::new(),
        };
        std::fs::write(&lock, me.text()).unwrap();
        remove_lock_of(&lock, &stopped).unwrap();
        assert!(lock.exists(), "a newer holder's lock was deleted");
        std::fs::write(&lock, "").unwrap();
        remove_lock_of(&lock, &me).unwrap();
        assert!(lock.exists(), "a lock being written was deleted");
        std::fs::write(&lock, me.text()).unwrap();
        remove_lock_of(&lock, &me).unwrap();
        assert!(!lock.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agora-fd-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_default_interface_is_the_browser_unless_there_is_no_screen() {
        let none = |_: &str| false;
        let ssh = |k: &str| k == "SSH_CONNECTION";
        assert_eq!(Ui::default_for(&ssh), Ui::Tui);
        if std::env::consts::OS == "linux" {
            assert_eq!(Ui::default_for(&none), Ui::Tui);
            assert_eq!(Ui::default_for(&|k: &str| k == "DISPLAY"), Ui::Web);
        } else {
            assert_eq!(Ui::default_for(&none), Ui::Web);
        }
    }

    #[test]
    fn the_machine_file_reads_back_what_it_wrote() {
        let m = Machine {
            roles: vec!["hub".into(), "node".into()],
            ui: Ui::Web,
            hub_listen: "0.0.0.0:7445".into(),
            hub_names: vec!["localhost".into(), "box".into(), "192.168.1.5".into()],
            hub_dir: PathBuf::from(r#"C:\Users\a "b"\.agora\hub"#),
            node_projects: vec![PathBuf::from("/home/a/proj"), PathBuf::from("/x\ty")],
        };
        assert_eq!(Machine::parse(&m.to_toml()), Some(m.clone()));
        assert_eq!(m.local_addr().as_deref(), Some("127.0.0.1:7445"));
        assert_eq!(m.own_hub_url().as_deref(), Some("wss://localhost:7445"));
        // Unknown keys are ignored; a hub without an address is not a usable file.
        let text = format!("{}future = \"x\"\n", m.to_toml());
        assert!(Machine::parse(&text).is_some());
        assert_eq!(
            Machine::parse("roles = [\"hub\"]\nhub_dir = \"/h\"\n"),
            None
        );
        assert_eq!(Machine::parse("roles = [\"hub\"\n"), None);
        assert_eq!(Machine::parse("ui = \"tui\"\n"), None);
        assert_eq!(Ui::parse("desktop"), None);
    }

    #[test]
    fn a_hostname_becomes_a_valid_node_id() {
        assert_eq!(node_id_from(Some("My Laptop.local\n".into())), "my-laptop");
        assert_eq!(node_id_from(Some("mac.lan".into())), "mac");
        assert_eq!(node_id_from(Some("--Box (2)".into())), "box--2");
        assert_eq!(node_id_from(None), "this-machine");
        assert_eq!(node_id_from(Some("x".repeat(80))).len(), 64);
    }

    #[test]
    fn the_first_run_asks_once_and_join_is_not_built_yet() {
        let home = tmp("ask");
        let repo = home.join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let mut out = Vec::new();

        // A wrong answer is asked again; 1 then "y" listens on every address with the machine's names.
        let mut input: &[u8] = b"9\n1\ny\n";
        let m = ask(&say(), &mut input, &mut out, &home, &repo, Ui::Tui)
            .unwrap()
            .unwrap();
        assert_eq!(m.roles, ["hub", "node"]);
        assert!(m.hub_listen.starts_with("0.0.0.0:"), "{}", m.hub_listen);
        assert_eq!(m.hub_names[0], "localhost");
        assert_eq!(m.node_projects, std::slice::from_ref(&repo));
        assert_eq!(m.hub_dir, home.join("hub"));
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.contains("Start Agora here"), "{text}");
        assert!(text.contains("other machines"), "{text}");

        // "Just this machine": loopback only, no LAN question; a folder that is not a checkout is not offered.
        let mut input: &[u8] = b"2\n";
        let m = ask(&say(), &mut input, &mut Vec::new(), &home, &home, Ui::None)
            .unwrap()
            .unwrap();
        assert!(m.hub_listen.starts_with("127.0.0.1:"), "{}", m.hub_listen);
        assert_eq!(m.hub_names, ["localhost"]);
        assert!(m.node_projects.is_empty());
        assert_eq!(m.ui, Ui::None);

        // "Start here" answered no: loopback.
        let mut input: &[u8] = b"1\n\n";
        let m = ask(&say(), &mut input, &mut Vec::new(), &home, &home, Ui::Tui)
            .unwrap()
            .unwrap();
        assert!(m.hub_listen.starts_with("127.0.0.1:"));

        // Join: a pointer to the manual way, nothing saved.
        let mut out = Vec::new();
        let mut input: &[u8] = b"3\n";
        assert_eq!(
            ask(&say(), &mut input, &mut out, &home, &home, Ui::Tui).unwrap(),
            None
        );
        assert!(String::from_utf8_lossy(&out).contains("agora node enroll"));

        // Input ended without an answer.
        let mut input: &[u8] = b"";
        assert!(ask(&say(), &mut input, &mut Vec::new(), &home, &home, Ui::Tui).is_err());
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_busy_port_is_skipped() {
        let taken = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = taken.local_addr().unwrap().port();
        let found = free_port(IpAddr::V4(Ipv4Addr::LOCALHOST), port).unwrap();
        assert_ne!(found, port);
        assert!(found > port);
    }

    #[test]
    fn a_stale_lock_is_replaced_and_a_live_one_is_kept() {
        let dir = tmp("lock");
        let path = dir.join("hub.lock");
        // A pid that ran and ended.
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        std::fs::write(&path, format!("{pid} 12345 0.0.1\n")).unwrap();
        assert!(matches!(claim(&path).unwrap(), Claim::Mine));
        let mine = read_lock(&path).unwrap();
        assert_eq!(mine.pid, std::process::id());
        assert!(mine.alive());
        // Held by a live process (this one): a second claim does not take it.
        assert!(matches!(claim(&path).unwrap(), Claim::Held));
        // This process's pid with another start time is a reused pid: stale.
        std::fs::write(&path, format!("{} 1 0.0.1\n", std::process::id())).unwrap();
        assert!(matches!(claim(&path).unwrap(), Claim::Mine));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_large_log_is_kept_once_as_dot_one() {
        let dir = tmp("rotate");
        let log = dir.join("hub.log");
        std::fs::write(&log, vec![b'x'; (LOG_LIMIT + 1) as usize]).unwrap();
        rotate(&log);
        assert!(!log.exists());
        assert!(dir.join("hub.log.1").exists());
        std::fs::write(&log, "small").unwrap();
        rotate(&log);
        assert!(log.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
