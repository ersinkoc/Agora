//! `agora` with no arguments: the front door (ADR-0062, rung 1 of `docs/map.md`).
//!
//! The first time, a short choice writes `<AGORA_HOME>/machine.toml` (what this machine runs; never a secret). Every time after, the
//! roles it remembers (a hub, then a node enrolled with that hub) are started **detached** when they are not running, and the
//! interface opens signed in: the terminal client by default, the browser with `agora web` or `--ui web`. Closing or killing the
//! interface changes nothing for the hub, the node or the agents: they are processes of their own. `agora status` shows them and
//! `agora down` stops what the front door started.
//!
//! A browser is opened only on a machine with a screen, outside CI and unless `AGORA_NO_BROWSER` is set ([`may_open_browser`]);
//! otherwise the address is printed. `agora web` needs no terminal for it (a desktop launcher or shortcut runs it with none); `agora
//! panel`, which runs until Ctrl+C, does. On a machine that only joined a hub elsewhere, `agora web` runs the hub's panel (`agora panel`, ADR-0056 Amendment A),
//! which holds the terminal while it runs and fails when the hub does not answer.

use std::io::{BufRead, IsTerminal, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use agora_i18n::{key, Arg};
use agora_node::link_state::{self, LinkState};
use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};
use agora_tui::client::{sign_in, Hub, Session, Update};

use crate::hubcli::{enroll_unless_linked, new_enrollment_code, new_login_code, EnrollFailure};
use crate::{home, Opts, Say};

pub(crate) const MACHINE_FILE: &str = "machine.toml";
const DEFAULT_PORT: u16 = 7443;
/// How long a started hub or node has to answer before the front door gives up and points at its log.
const WAIT: Duration = Duration::from_secs(20);
/// A log larger than this at start is renamed to `<name>.1` (replacing the previous one) and a new one begun.
pub(crate) const LOG_LIMIT: u64 = 5 * 1024 * 1024;

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
    /// ADR-0062 item 4: whether the node brings back by itself the agents that were running when it stopped (`resume = "auto"`) or
    /// only offers them (`"ask"`, also any other value). `None`: never answered (a file written before this setting existed), which
    /// is ask until the owner answers the question at the next `agora` in a terminal: an upgrade never turns it on silently.
    pub resume: Option<bool>,
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
        crate::hubcli::local_dial_addr(&self.hub_listen)
            .or_else(|| Some(format!("127.0.0.1:{}", self.port()?)))
    }

    /// The hub address a node enrolled with this machine's own hub keeps in its link folder.
    fn own_hub_url(&self) -> Option<String> {
        Some(format!("wss://localhost:{}", self.port()?))
    }

    /// The settings of a machine that joined a hub elsewhere (ADR-0065): it runs a node and no hub, and offers no folder yet (any folder
    /// is chosen later in the interface). `ui` is the system default, as for a first run.
    pub(crate) fn joined(home: &Path) -> Machine {
        Machine {
            roles: vec!["node".into()],
            ui: Ui::system_default(),
            hub_listen: String::new(),
            hub_names: Vec::new(),
            hub_dir: home.join("hub"),
            node_projects: Vec::new(),
            resume: None,
        }
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
             node_projects = {}\n{}",
            list(&self.roles),
            quote(self.ui.as_str()),
            quote(&self.hub_listen),
            list(&self.hub_names),
            quote(&self.hub_dir.to_string_lossy()),
            list(&paths),
            self.resume.map_or_else(String::new, |r| format!(
                "resume = {}\n",
                quote(if r { "auto" } else { "ask" })
            )),
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
        // ADR-0062 item 4: not answered unless the file says (a file from before the setting was never asked).
        let mut resume = None;
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
                // Only "auto" is yes: an unknown word is the stricter answer.
                "resume" => resume = Some(parse_string(v)?.0 == "auto"),
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
            resume,
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
    let path = machine_path(home);
    let text = std::fs::read_to_string(&path).ok()?;
    // Issue #877: an older version (or a `D:\` ACL) left it open, and another local user could change what this machine runs. It
    // is written owner-only today (ADR-0062); one left open is closed the next time it is read. Best effort.
    let _ = agora_platform::close_to_others(&path);
    Machine::parse(&text)
}

/// Saves `m` unless the machine already has settings (a joined machine never overwrites what `agora` set up).
pub(crate) fn save_if_absent(home: &Path, m: &Machine) -> std::io::Result<()> {
    if machine_path(home).exists() {
        return Ok(());
    }
    save(home, m)
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
pub(crate) fn node_id_from(hostname: Option<String>) -> String {
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

/// Whether something accepts connections at `addr`.
fn answers(addr: SocketAddr) -> bool {
    TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok()
}

/// The first port from `from` that `ip` can listen on and nobody answers on.
fn free_port(ip: IpAddr, from: u16) -> Option<u16> {
    (from..from.saturating_add(100)).find(|p| {
        if answers(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), *p)) {
            return false;
        }
        // One at a time: each test listener is closed before the next is tried.
        let wanted = TcpListener::bind(SocketAddr::new(ip, *p)).is_ok();
        let loopback =
            TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), *p)).is_ok();
        wanted && loopback
    })
}

/// The first-run choice.
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

/// Asks the first-run questions and builds the machine's settings (`None`: the person chose to join and gave no address).
fn ask(
    say: &Say,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    home: &Path,
    cwd: &Path,
    ui: Ui,
    resume: Option<bool>,
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
            // ADR-0065: the person types the hub's address; with none, they are told that `agora join` finds hubs by itself.
            prompt(out, &say.text(key!("frontdoor-join-address"), &[]));
            let typed = read_answer(input).unwrap_or_default();
            if typed.is_empty() {
                let _ = writeln!(out, "{}", say.text(key!("join-help"), &[]));
                return Ok(None);
            }
            return match crate::joincmd::run(
                say,
                &typed,
                None,
                false,
                None,
                &agora_node::pairing_client::Timeouts::default(),
            ) {
                Ok(_) => {
                    // A machine that only runs a node: the rest of the front door starts it and keeps it running.
                    let mut m = Machine::joined(home);
                    m.ui = ui;
                    m.resume = resume;
                    if cwd.join(".git").exists() {
                        m.node_projects = vec![cwd.to_path_buf()];
                    }
                    Ok(Some(m))
                }
                Err(stop) => Err(stop.code()),
            };
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
    // ADR-0062 item 4: asked once, unless `--resume` already answered.
    let resume = resume.or_else(|| ask_resume(say, input, out));
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
        resume,
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
    // ADR-0062 item 4: `--resume ask|auto` for this run (the node's supervisor gets it), kept with `--save`.
    let resume = match o.resume.as_deref() {
        None => None,
        Some("auto") => Some(true),
        Some("ask") => Some(false),
        Some(_) => {
            say.err(key!("cli-bad-arguments"), &[]);
            return ExitCode::from(2);
        }
    };
    // `--listen ADDRESS`: where the hub listens (for a machine reached from outside, ADR-0067). The supervisor reads it from
    // `machine.toml`, so it is always saved.
    let listen = match o.listen.as_deref().map(str::parse::<SocketAddr>) {
        None => None,
        Some(Ok(l)) => Some(l.to_string()),
        Some(Err(_)) => {
            say.err(key!("cli-bad-arguments"), &[]);
            return ExitCode::from(2);
        }
    };
    let h = home();
    let mut first_run = false;
    let m = match load(&h) {
        Some(mut m) => {
            let mut changed = false;
            if let Some(l) = listen
                .as_ref()
                .filter(|l| m.has("hub") && **l != m.hub_listen)
            {
                m.hub_listen.clone_from(l);
                changed = true;
                say.out(
                    key!("frontdoor-listen-saved"),
                    &[("addr", l.clone().into())],
                );
            }
            if let Some(u) = ui.filter(|u| o.save && *u != m.ui) {
                m.ui = u;
                changed = true;
            }
            let mut resume_saved = false;
            if let Some(r) = resume.filter(|r| o.save && m.resume != Some(*r)) {
                m.resume = Some(r);
                changed = true;
                resume_saved = true;
            }
            // ADR-0062 item 4: a machine set up before the question existed is asked once, at the first `agora` in a terminal; until
            // then (and when the input ends without an answer) nothing comes back by itself.
            if m.resume.is_none() && resume.is_none() && std::io::stdin().is_terminal() {
                if let Some(r) =
                    ask_resume(say, &mut std::io::stdin().lock(), &mut std::io::stdout())
                {
                    m.resume = Some(r);
                    changed = true;
                    resume_saved = true;
                }
            }
            if changed {
                if let Err(e) = save(&h, &m) {
                    return failed(say, &e.to_string());
                }
            }
            if resume_saved {
                // A node running now (and its supervisor, for a restart after a crash) keeps what it started with.
                say.out(key!("frontdoor-resume-saved"), &[]);
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
                resume,
            );
            match asked {
                Ok(Some(mut m)) => {
                    if let Some(l) = listen.filter(|_| m.has("hub")) {
                        m.hub_listen = l;
                    }
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
    if let Err(code) = up(say, &h, &m, resume) {
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
    open(say, &h, &m, ui.unwrap_or(m.ui))
}

/// ADR-0062 item 4: "Bring agents back by themselves after a restart? [Y/n]". Enter or yes is yes (the default the maintainer chose),
/// a no is no and says how to turn it on again; input that ended is no answer (`None`: asked again next time, nothing comes back by
/// itself meanwhile).
fn ask_resume(say: &Say, input: &mut dyn BufRead, out: &mut dyn Write) -> Option<bool> {
    prompt(out, &say.text(key!("frontdoor-resume-question"), &[]));
    let yes = wants_resume(read_answer(input).as_deref()?);
    if !yes {
        let _ = writeln!(out, "{}", say.text(key!("frontdoor-resume-off"), &[]));
    }
    Some(yes)
}

/// Only a no is a no: Enter, yes, or anything else is yes.
fn wants_resume(answer: &str) -> bool {
    !matches!(answer, "n" | "no" | "h" | "hayır" | "hayir")
}

/// The answer to "Start Agora when you log in? [Y/n]": Enter or yes is yes, no is no, and no answer at all (the input ended) is no.
fn wants_autostart(answer: Option<&str>) -> bool {
    matches!(answer, Some("" | "y" | "yes" | "e" | "evet"))
}

/// `agora web`: makes sure this machine's Agora runs, then opens it in the browser, signed in. A machine that only joined a hub
/// elsewhere opens the hub's panel through its node (ADR-0056 Amendment A): `agora web` there is `agora panel`, and runs while it does.
pub fn web(say: &Say) -> ExitCode {
    let h = home();
    let Some(m) = load(&h) else {
        say.err(key!("frontdoor-not-set-up"), &[]);
        return ExitCode::from(1);
    };
    if let Err(code) = up(say, &h, &m, None) {
        return code;
    }
    if !m.has("hub") && m.has("node") {
        return crate::panelcmd::panel(say);
    }
    open(say, &h, &m, Ui::Web)
}

/// The variable that keeps every Agora command from opening a browser by itself (it prints the address instead).
pub(crate) const NO_BROWSER_ENV: &str = "AGORA_NO_BROWSER";

/// Whether a command may open a browser by itself: on a machine with a screen to show it on (not over SSH, not Linux without a display:
/// [`Ui::default_for`]), outside CI, unless [`NO_BROWSER_ENV`] says no, and, for a command that `needs_terminal`, only for a person at a
/// terminal (`at_terminal`). `agora web` does not need one: a desktop launcher or shortcut runs it without a terminal and must still open
/// the browser. `env` says whether a variable is set to something. Otherwise the command prints the address, which it does in any case.
pub(crate) fn may_open_browser(
    needs_terminal: bool,
    at_terminal: bool,
    env: &dyn Fn(&str) -> bool,
) -> bool {
    (at_terminal || !needs_terminal)
        && !env("CI")
        && !env(NO_BROWSER_ENV)
        && Ui::default_for(env) == Ui::Web
}

/// [`may_open_browser`] for this process: whether standard input is a terminal, and this process's environment.
pub(crate) fn may_open_browser_here(needs_terminal: bool) -> bool {
    may_open_browser(needs_terminal, std::io::stdin().is_terminal(), &|k| {
        std::env::var_os(k).is_some_and(|v| !v.is_empty())
    })
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

fn open(say: &Say, home: &Path, m: &Machine, ui: Ui) -> ExitCode {
    let (Some(addr), true) = (hub_addr(home, m), m.has("hub")) else {
        // A machine that joined a hub elsewhere has nothing to open here: its window is the hub's.
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
            let Some(port) = addr.parse::<SocketAddr>().ok().map(|a| a.port()) else {
                return failed(say, &m.hub_listen);
            };
            // The code is in the fragment, which the browser never sends; the page trades it for a session at once.
            let url = format!("https://localhost:{port}/#code={code}");
            // No terminal needed: a desktop launcher or shortcut runs `agora web` without one.
            if may_open_browser_here(false) && agora_platform::open_url(&url).is_ok() {
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

/// Where this machine reaches its own hub now. A hub that runs (its lock's holder is alive) is reached where it wrote that it listens
/// (`<hub dir>/listen`, when written since that lock was): `agora --listen` changes `hub_listen` only from the next start on, so the
/// running hub may still be at the address before. Otherwise, and before a new hub has written it, where `hub_listen` names.
fn hub_addr(home: &Path, m: &Machine) -> Option<String> {
    let modified = |p: &Path| std::fs::metadata(p).and_then(|d| d.modified()).ok();
    let record = m.hub_dir.join(crate::hubcli::LISTEN_FILE);
    let running =
        holder(home, Role::Hub).is_some() && modified(&record) >= modified(&Role::Hub.lock(home));
    // Its port, on loopback, as `local_addr` reaches it.
    running
        .then(|| crate::hubcli::own_hub_addr(&m.hub_dir))
        .flatten()
        .or_else(|| m.local_addr())
}

/// Whether the role answers: the hub at its address, the node on its control socket.
pub(crate) fn role_answers(home: &Path, m: &Machine, role: Role) -> bool {
    match role {
        Role::Hub => hub_addr(home, m)
            .and_then(|a| a.parse().ok())
            .is_some_and(answers),
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
            // ADR-0062 item 4: the agents that were running come back when the node starts again (also under a supervisor or a
            // service, which run this same command).
            if m.resume == Some(true) {
                cmd.args(["--resume", "auto"]);
            }
        }
    }
    Ok(cmd)
}

/// The command line of `role`'s supervisor (`agora supervise <role>`), which runs [`command`] and starts it again after a crash.
/// The supervisor waits only for this process (`--handoff PID:START`, the holder of the lock now) to hand the lock over; any other
/// holder makes it refuse at once, so a supervisor never takes a lock that `agora down` just freed.
/// `resume` (ADR-0062 item 4): `--resume auto|ask` for this run instead of the machine file's setting.
fn supervisor_command(
    home: &Path,
    role: Role,
    resume: Option<bool>,
) -> std::io::Result<std::process::Command> {
    let me = LockInfo::of(std::process::id())
        .ok_or_else(|| std::io::Error::other("own start time unknown"))?;
    let mut cmd = std::process::Command::new(std::env::current_exe()?);
    cmd.env("AGORA_HOME", home)
        .current_dir(home)
        .args(["supervise", role.name(), "--handoff"])
        .arg(format!("{}:{}", me.pid, me.start));
    if let Some(r) = resume {
        cmd.args(["--resume", if r { "auto" } else { "ask" }]);
    }
    Ok(cmd)
}

/// Starts `role` detached under its supervisor unless it runs already, then waits for it to answer. The lock is this process's
/// until the supervisor's id replaces it; the supervisor waits for that before it starts the role.
fn ensure(
    say: &Say,
    home: &Path,
    m: &Machine,
    role: Role,
    resume: Option<bool>,
) -> Result<(), ExitCode> {
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
            let started = supervisor_command(home, role, resume)
                .and_then(|mut c| agora_platform::spawn_detached(&mut c, &log))
                .map(|d| {
                    if d.in_job {
                        // Windows: the terminal's job would not let it go, so closing that terminal ends it. Said here and in its log.
                        let text = say.text(
                            key!("frontdoor-in-job"),
                            &[("role", role.label(say).into())],
                        );
                        crate::err_line(&text);
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
/// `resume`: `agora --resume ask|auto` for this run, passed to the node's supervisor (only a node started now uses it).
fn up(say: &Say, home: &Path, m: &Machine, resume: Option<bool>) -> Result<(), ExitCode> {
    for d in ["logs", "run"] {
        std::fs::create_dir_all(home.join(d)).map_err(|e| failed(say, &e.to_string()))?;
    }
    if m.has("hub") {
        // ADR-0067: the hub is never started on a public address without the screen password; said here, in words, instead of in a log.
        crate::remotecmd::check_listen(say, &m.hub_dir, &m.hub_listen)?;
        ensure(say, home, m, Role::Hub, None)?;
    }
    if m.has("node") {
        if m.has("hub") && !crate::hubcli::link_dir().join("node-id").exists() {
            enroll_self(say, m)?;
        }
        ensure(say, home, m, Role::Node, resume)?;
        if m.has("hub") {
            wait_linked(say, home, m)?;
        }
    }
    Ok(())
}

/// Enrolls this machine's node with this machine's own hub: a one-time code made here and used at once, never shown. Nothing is enrolled
/// when the link folder got a link since `up` looked (another `agora` started at the same time, audit F1030): the look is made again,
/// and the enrollment and the save follow it, under the folder's guard.
fn enroll_self(say: &Say, m: &Machine) -> Result<(), ExitCode> {
    let (Some(url), Ok(ca_pem)) = (
        m.own_hub_url(),
        std::fs::read_to_string(m.hub_dir.join("ca.pem")),
    ) else {
        return Err(failed(say, &m.hub_dir.join("ca.pem").display().to_string()));
    };
    let code = new_enrollment_code(&m.hub_dir).map_err(|e| failed(say, &e))?;
    let id = node_id_from(agora_platform::hostname());
    match enroll_unless_linked(&url, &ca_pem, &code, &id) {
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
    let (Some(node_id), Some(addr), Ok(ca_pem)) = (
        read("node-id"),
        hub_addr(home, m),
        std::fs::read_to_string(m.hub_dir.join("ca.pem")),
    ) else {
        return Ok(());
    };
    // A node linked to another hub is not waited for here. This hub's own node is linked at its saved port, or at the port the
    // running hub has while a `--listen` change waits for its next start.
    let running_url = addr
        .parse::<SocketAddr>()
        .ok()
        .map(|a| format!("wss://localhost:{}", a.port()));
    let url = read("url");
    if url != m.own_hub_url() && url != running_url {
        return Ok(());
    }
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
    crate::out_line(&format!("  {}", say.text(id, args)));
}

/// A line already rendered, indented under its role.
fn detail_text(line: &str) {
    crate::out_line(&format!("  {line}"));
}

/// The node's hub line in `agora status`. `link`: the hub and the id this machine is enrolled as, `None` when it is not enrolled.
/// `state`: `None` while the node is not running (nothing is said about a link that cannot be up), `Some(None)` for a node that
/// does not report its link (one started by an earlier version). "Connected" only for a link that is up now, never for the hub the
/// machine is merely configured for. `macos`: the pointer to the Local Network permission is added where the system blocks the way.
pub(crate) fn link_line(
    say: &Say,
    link: Option<(String, String)>,
    state: Option<Option<LinkState>>,
    macos: bool,
) -> Option<String> {
    let Some((hub, id)) = link else {
        return Some(say.text(key!("status-node-not-linked"), &[]));
    };
    let args = |reason: &str| -> Vec<(&str, Arg)> {
        vec![
            ("hub", hub.clone().into()),
            ("reason", reason.to_string().into()),
        ]
    };
    Some(match state? {
        None => say.text(key!("status-node-link-unknown"), &args("")),
        Some(LinkState::Up) => say.text(
            key!("status-node-linked"),
            &[("hub", hub.clone().into()), ("id", id.into())],
        ),
        Some(LinkState::Connecting) => say.text(key!("status-node-connecting"), &args("")),
        Some(LinkState::Down { code, param }) => {
            let key = match (code, macos) {
                (link_state::UNREACHABLE, true) => key!("status-node-link-unreachable-macos"),
                (link_state::UNREACHABLE, false) => key!("status-node-link-unreachable"),
                (link_state::NOT_PERMITTED, true) => key!("status-node-link-not-permitted-macos"),
                (link_state::NOT_PERMITTED, false) => key!("status-node-link-not-permitted"),
                (link_state::REFUSED, _) => key!("status-node-link-refused"),
                (link_state::CERTIFICATE, _) => key!("status-node-link-certificate"),
                (link_state::HUB_REFUSED, _) => key!("status-node-link-hub-refused"),
                (link_state::CLOSED, _) => key!("status-node-link-closed"),
                _ => key!("status-node-link-other"),
            };
            say.text(key, &args(&param))
        }
    })
}

/// `agora status`: one screen about what this machine runs.
pub fn status(say: &Say, o_data: &Option<PathBuf>) -> ExitCode {
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
                    crate::out_line(&crate::supervise::restarting_line(say, role, *pid, r));
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
                let running = held.is_some() || answers;
                let data = o_data.clone().unwrap_or_else(|| h.join("data"));
                if let Some(line) = link_line(
                    say,
                    read("url").zip(read("node-id")),
                    running.then(|| LinkState::read(&data)),
                    agora_platform::SERVICES_NEED_NETWORK_PERMISSION_AFTER_UPDATE,
                ) {
                    detail_text(&line);
                }
                if let Some(n) = agents_running(&h) {
                    detail(say, key!("status-agents"), &[("count", n.into())]);
                } else if let Some((_, n)) = crate::hold::read_info(&h).filter(|(_, n)| *n > 0) {
                    // ADR-0079: the node is not running, the agents are.
                    detail(
                        say,
                        key!("status-agents-kept"),
                        &[("count", (n as i64).into())],
                    );
                }
                // ADR-0093: a holder from before 0.3.27 cannot keep structured agents across a node restart.
                if let Some(old) = agora_node::holder_note::old_holder(&h) {
                    detail(
                        say,
                        key!("status-holder-old"),
                        &[("count", (old.kept as i64).into())],
                    );
                }
            }
        }
        for line in crate::supervise::status_lines(say, &h, role, held.is_some(), answers) {
            crate::out_line(&format!("  {line}"));
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
    // The lock is believed only in a folder its guard accepts (this user's, not a link): one that is not is refused before the
    // process it names is stopped, as a start refuses it, never after.
    if let Some(dir) = lock
        .parent()
        .filter(|d| std::fs::symlink_metadata(d).is_ok())
    {
        if let Err(e) = agora_platform::ensure_private_dir(dir) {
            *code = down_lock_failed(say, &lock, &e);
            return false;
        }
    }
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

/// ADR-0079: what `agora down` does about the terminal holder, after the node is stopped. The holder keeps the agents running
/// (plain `agora down` says how many); with `agents` it is asked to end them (`SIGTERM`: it ends every agent's group, then itself).
fn stop_holder(say: &Say, h: &Path, agents: bool, code: &mut ExitCode) -> bool {
    let Some((pid, running)) = crate::hold::read_info(h) else {
        return false;
    };
    if !agents {
        if running == 0 {
            return false;
        }
        say.out(
            key!("down-agents-kept"),
            &[("count", (running as i64).into())],
        );
        return true;
    }
    let failed = |code: &mut ExitCode, reason: Arg| {
        say.err(
            key!("down-failed"),
            &[
                ("role", say.text(key!("down-holder"), &[]).into()),
                ("reason", reason),
            ],
        );
        *code = ExitCode::from(1);
    };
    if let Err(e) = agora_platform::terminate_process(pid) {
        failed(code, e.to_string().into());
        return false;
    }
    // The holder gives its agents a few seconds before it kills them.
    let end = Instant::now() + Duration::from_secs(10);
    while crate::hold::read_info(h).is_some() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(100));
    }
    if crate::hold::read_info(h).is_some() {
        failed(code, say.text(key!("down-still-running"), &[]).into());
        return false;
    }
    say.out(
        key!("down-agents-stopped"),
        &[("count", (running as i64).into())],
    );
    true
}

/// `agora down`: stops what the front door started, the node first, then the hub. Agents the terminal holder keeps running (ADR-0079)
/// keep running, and `agents` (`--agents`) stops them too.
pub fn down(say: &Say, agents: bool) -> ExitCode {
    let h = home();
    let machine = load(&h);
    let mut code = ExitCode::SUCCESS;
    let mut stopped_any = false;
    for role in [Role::Node, Role::Hub] {
        stopped_any |= stop_role(say, &h, machine.as_ref(), role, &mut code);
    }
    stopped_any |= stop_holder(say, &h, agents, &mut code);
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

    fn line(state: Option<Option<LinkState>>, macos: bool) -> Option<String> {
        link_line(
            &say(),
            Some(("wss://hub.example:7444".into(), "mac".into())),
            state,
            macos,
        )
    }

    /// "Connected" is only for a link that is up now; every other state says not connected (the bug: it was printed for the
    /// configured hub whatever the link did).
    #[test]
    fn status_says_connected_only_for_a_link_that_is_up() {
        let up = line(Some(Some(LinkState::Up)), false).unwrap();
        assert!(
            up.starts_with("connected to wss://hub.example:7444 as mac"),
            "{up}"
        );
        let route = LinkState::down_transport("No route to host (os error 65)");
        for state in [
            Some(Some(LinkState::Connecting)),
            Some(None),
            Some(Some(route.clone())),
            Some(Some(LinkState::down_transport(
                "Connection refused (os error 61)",
            ))),
            Some(Some(LinkState::down_transport(
                "invalid peer certificate: UnknownIssuer",
            ))),
            Some(Some(LinkState::down_hub_refused("auth.unauthenticated"))),
            Some(Some(LinkState::down(link_state::CLOSED, ""))),
            Some(Some(LinkState::down_transport("something new"))),
        ] {
            let t = line(state.clone(), false).unwrap();
            assert!(!t.starts_with("connected to"), "{state:?}: {t}");
            assert!(
                t.contains("not connected to") || t.contains("is not known"),
                "{t}"
            );
            assert!(!t.contains(" as mac"), "{t}");
        }
        // A node that is not running says nothing about a link; a machine that is not enrolled says so.
        assert_eq!(line(None, false), None);
        assert_eq!(
            link_line(&say(), None, Some(Some(LinkState::Up)), false).unwrap(),
            "not connected to a hub"
        );
    }

    #[test]
    fn status_shows_the_reason_and_points_to_local_network_only_on_macos() {
        let route = Some(Some(LinkState::down_transport(
            "No route to host (os error 65)",
        )));
        let mac = line(route.clone(), true).unwrap();
        assert!(mac.contains("No route to host (os error 65)"), "{mac}");
        assert!(
            mac.contains("Local Network") && mac.contains("restart the node once more"),
            "{mac}"
        );
        let other = line(route, false).unwrap();
        assert!(
            other.contains("os error 65") && !other.contains("Local Network"),
            "{other}"
        );
        let refused = line(
            Some(Some(LinkState::down_transport(
                "Connection refused (os error 61)",
            ))),
            true,
        )
        .unwrap();
        assert!(
            refused.contains("nothing is listening") && !refused.contains("Local Network"),
            "{refused}"
        );
        let tr = link_line(
            &Say(Catalog::for_locale("tr")),
            Some(("wss://h:1".into(), "m".into())),
            Some(Some(LinkState::down_transport(
                "No route to host (os error 65)",
            ))),
            true,
        )
        .unwrap();
        assert!(
            tr.contains("Yerel Ağ") && tr.contains("yeniden başlatın"),
            "{tr}"
        );
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
            // The guard waits 10 s. A runner that stalls the whole process longer than that (seen on Ubuntu CI: a 14 s gap) lets
            // the waiting thread run first when it resumes, past its deadline while the guard is still held: TimedOut, although
            // nothing is wrong. Tried again a bounded number of times; one that never gets the guard still fails.
            std::thread::spawn(move || {
                for _ in 0..3 {
                    match remove_lock_of(&lock, &stopped) {
                        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                        r => return r.unwrap(),
                    }
                }
                panic!("the guard was never let go");
            })
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

    /// `agora down` refused a `run` folder that is a link only after it had stopped the process a lock there names: through a link
    /// to another Agora home's `run`, it stopped that home's hub. Now it is refused first (a start refuses it too) and the process
    /// and its lock stay; the same lock in a plain folder is stopped.
    #[test]
    fn down_refuses_a_linked_run_folder_before_stopping_anything() {
        let dir = tmp("run-link");
        let (home, elsewhere, plain) = (dir.join("home"), dir.join("elsewhere"), dir.join("plain"));
        for d in [&home, &elsewhere, &plain.join("run")] {
            std::fs::create_dir_all(d).unwrap();
        }
        // A live process for the lock to name: this test binary again, waiting in `supervise`'s child helper.
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "supervise::tests::child_helper",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("AGORA_TEST_CHILD", "60000 0")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let held = LockInfo::of(child.id()).unwrap();
        std::fs::write(elsewhere.join("hub.lock"), held.text()).unwrap();
        agora_platform::link_dir(&elsewhere, &home.join("run")).unwrap();

        let mut code = ExitCode::SUCCESS;
        let stopped = stop_role(&say(), &home, None, Role::Hub, &mut code);
        let (survived, lock_kept) = (held.alive(), elsewhere.join("hub.lock").exists());

        // Control: the same lock in a plain `run` folder is believed, and its process is stopped.
        std::fs::write(plain.join("run").join("hub.lock"), held.text()).unwrap();
        let mut plain_code = ExitCode::SUCCESS;
        let plain_stopped = stop_role(&say(), &plain, None, Role::Hub, &mut plain_code);
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(dir);

        assert!(
            !stopped && survived,
            "a lock behind a linked run folder was acted on"
        );
        assert!(lock_kept);
        assert!(code == ExitCode::from(1));
        assert!(plain_stopped && plain_code == ExitCode::SUCCESS);
    }

    /// `agora --listen` changes the hub's address from its next start on, but the front door then looked for the running hub at the
    /// new address ("did not start", exit 1). A running hub is reached where it wrote that it listens, unless that record is older
    /// than the lock it runs under (left by the hub before); with no running hub, `hub_listen` decides.
    #[test]
    fn the_running_hub_is_reached_where_it_listens() {
        let home = tmp("hub-addr");
        let m = Machine {
            roles: vec!["hub".into()],
            ui: Ui::None,
            hub_listen: "127.0.0.1:7446".into(),
            hub_names: vec!["localhost".into()],
            hub_dir: home.join("hub"),
            node_projects: vec![],
            resume: None,
        };
        std::fs::create_dir_all(&m.hub_dir).unwrap();
        std::fs::create_dir_all(home.join("run")).unwrap();
        let record = m.hub_dir.join(crate::hubcli::LISTEN_FILE);
        let lock = Role::Hub.lock(&home);
        let touch = |p: &Path, t: std::time::SystemTime| {
            std::fs::File::options()
                .write(true)
                .open(p)
                .unwrap()
                .set_modified(t)
                .unwrap();
        };
        let at = std::time::SystemTime::now();
        let secs = Duration::from_secs(10);

        // No running hub: where `hub_listen` names, also with a record from a hub before.
        assert_eq!(hub_addr(&home, &m).as_deref(), Some("127.0.0.1:7446"));
        std::fs::write(&record, "127.0.0.1:7445").unwrap();
        assert_eq!(hub_addr(&home, &m).as_deref(), Some("127.0.0.1:7446"));
        // Running (the lock names this live process) and the record written after the lock: the record.
        std::fs::write(&lock, LockInfo::of(std::process::id()).unwrap().text()).unwrap();
        touch(&lock, at);
        touch(&record, at + secs);
        assert_eq!(hub_addr(&home, &m).as_deref(), Some("127.0.0.1:7445"));
        // Its port, reached on loopback, as `hub_listen`'s is.
        std::fs::write(&record, "0.0.0.0:7447").unwrap();
        touch(&record, at + secs);
        assert_eq!(hub_addr(&home, &m).as_deref(), Some("127.0.0.1:7447"));
        // A record older than the lock is a hub before's: not believed.
        touch(&record, at - secs);
        assert_eq!(hub_addr(&home, &m).as_deref(), Some("127.0.0.1:7446"));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn hub_addr_and_local_addr_preserve_ipv6_loopback() {
        let mut m = Machine::joined(&tmp("ipv6-addr"));
        m.hub_listen = "[::1]:7448".into();
        assert_eq!(m.local_addr().as_deref(), Some("[::1]:7448"));
        m.hub_listen = "[::]:7449".into();
        assert_eq!(m.local_addr().as_deref(), Some("[::1]:7449"));
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agora-fd-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Issue #877: a `machine.toml` an older version (or an open volume ACL) left readable and writable by others is closed the next
    /// time it is read.
    #[cfg(unix)]
    #[test]
    fn a_machine_file_left_open_is_closed_when_read() {
        use std::os::unix::fs::PermissionsExt;
        let h = tmp("machine-open");
        save(&h, &Machine::joined(&h)).unwrap();
        let path = machine_path(&h);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(load(&h).is_some());
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let _ = std::fs::remove_dir_all(h);
    }

    #[test]
    fn a_browser_is_opened_only_on_a_machine_with_a_screen_and_by_the_panel_only_at_a_terminal() {
        let env = |set: &'static [&'static str]| move |k: &str| set.contains(&k);
        let desktop: &[&str] = &["DISPLAY", "WAYLAND_DISPLAY"];
        for needs_terminal in [false, true] {
            assert!(may_open_browser(needs_terminal, true, &env(desktop)));
            // Negative controls, one reason each: CI, the opt-out, SSH, and Linux with no display; for every command.
            for refused in [
                &["DISPLAY", "CI"][..],
                &["DISPLAY", NO_BROWSER_ENV],
                &["DISPLAY", "SSH_CONNECTION"],
            ] {
                assert!(
                    !may_open_browser(needs_terminal, true, &env(refused)),
                    "{refused:?}"
                );
            }
            if std::env::consts::OS == "linux" {
                assert!(!may_open_browser(needs_terminal, true, &env(&[])));
            }
        }
        // Without a terminal: `agora web` (a desktop launcher or shortcut) still opens it; `agora panel` does not.
        assert!(may_open_browser(false, false, &env(desktop)));
        assert!(!may_open_browser(true, false, &env(desktop)));
        // Without a terminal the other gates still hold for `agora web`.
        assert!(!may_open_browser(false, false, &env(&["DISPLAY", "CI"])));
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
            resume: Some(false),
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
    fn agents_come_back_unless_the_owner_said_no() {
        // ADR-0062 item 4: a file from before the setting was never asked (no resume by itself); only "auto" is yes; the node command
        // carries it.
        let yes = Machine {
            roles: vec!["hub".into(), "node".into()],
            ui: Ui::Tui,
            hub_listen: "127.0.0.1:7445".into(),
            hub_names: vec!["localhost".into()],
            hub_dir: PathBuf::from("/h"),
            node_projects: vec![],
            resume: Some(true),
        };
        let old = Machine {
            resume: None,
            ..yes.clone()
        };
        assert!(!old.to_toml().contains("resume"), "{}", old.to_toml());
        assert_eq!(Machine::parse(&old.to_toml()), Some(old.clone()));
        assert_eq!(Machine::parse(&yes.to_toml()), Some(yes.clone()));
        let no = Machine {
            resume: Some(false),
            ..yes.clone()
        };
        assert!(no.to_toml().contains("resume = \"ask\""));
        assert_eq!(Machine::parse(&no.to_toml()), Some(no.clone()));
        let odd = yes.to_toml().replace("\"auto\"", "\"always\"");
        assert_eq!(Machine::parse(&odd), Some(no.clone()));

        let args = |m: &Machine, role: Role| -> Vec<String> {
            command(Path::new("/h"), m, role)
                .unwrap()
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect()
        };
        assert!(args(&yes, Role::Node).ends_with(&["--resume".into(), "auto".into()]));
        assert!(!args(&no, Role::Node).contains(&"--resume".into()));
        assert!(!args(&old, Role::Node).contains(&"--resume".into()));
        assert!(!args(&yes, Role::Hub).contains(&"--resume".into()));

        // `agora --resume ask|auto` reaches the node's supervisor for this run only.
        let sup = |r: Option<bool>| -> Vec<String> {
            supervisor_command(Path::new("/h"), Role::Node, r)
                .unwrap()
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect()
        };
        assert!(!sup(None).contains(&"--resume".into()));
        assert!(sup(Some(false)).ends_with(&["--resume".into(), "ask".into()]));
        assert!(sup(Some(true)).ends_with(&["--resume".into(), "auto".into()]));

        for a in ["", "y", "evet", "maybe"] {
            assert!(wants_resume(a), "{a}");
        }
        for a in ["n", "no", "h", "hayır"] {
            assert!(!wants_resume(a), "{a}");
        }
        // Input that ended is no answer: asked again next time.
        let mut input: &[u8] = b"";
        assert_eq!(ask_resume(&say(), &mut input, &mut Vec::new()), None);
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
    fn the_first_run_asks_once_and_join_without_an_address_explains() {
        let home = tmp("ask");
        let repo = home.join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let mut out = Vec::new();

        // A wrong answer is asked again; 1 then "y" listens on every address with the machine's names.
        let mut input: &[u8] = b"9\n1\ny\n";
        let m = ask(&say(), &mut input, &mut out, &home, &repo, Ui::Tui, None)
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
        let m = ask(
            &say(),
            &mut input,
            &mut Vec::new(),
            &home,
            &home,
            Ui::None,
            None,
        )
        .unwrap()
        .unwrap();
        assert!(m.hub_listen.starts_with("127.0.0.1:"), "{}", m.hub_listen);
        assert_eq!(m.hub_names, ["localhost"]);
        assert!(m.node_projects.is_empty());
        assert_eq!(m.ui, Ui::None);

        // "Start here" answered no: loopback. Enter at the resume question keeps agents coming back.
        let mut input: &[u8] = b"1\n\n\n";
        let m = ask(
            &say(),
            &mut input,
            &mut Vec::new(),
            &home,
            &home,
            Ui::Tui,
            None,
        )
        .unwrap()
        .unwrap();
        assert!(m.hub_listen.starts_with("127.0.0.1:"));
        assert_eq!(m.resume, Some(true));

        // ADR-0062 item 4: asked once; a no is kept and says how to turn it on again.
        let mut out = Vec::new();
        let mut input: &[u8] = b"2\nn\n";
        let m = ask(&say(), &mut input, &mut out, &home, &home, Ui::Tui, None)
            .unwrap()
            .unwrap();
        assert_eq!(m.resume, Some(false));
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.contains("after a restart"), "{text}");
        assert!(text.contains("agora --resume auto --save"), "{text}");
        // `--resume` already answered: no question.
        let mut out = Vec::new();
        let mut input: &[u8] = b"2\n";
        let m = ask(
            &say(),
            &mut input,
            &mut out,
            &home,
            &home,
            Ui::Tui,
            Some(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(m.resume, Some(false));
        assert!(!String::from_utf8_lossy(&out).contains("after a restart"));

        // Join with no address: how to get one, nothing saved.
        let mut out = Vec::new();
        let mut input: &[u8] = b"3\n\n";
        assert_eq!(
            ask(&say(), &mut input, &mut out, &home, &home, Ui::Tui, None).unwrap(),
            None
        );
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.contains("agora pair open"), "{text}");
        assert!(text.contains("agora join"), "{text}");

        // Join with an address nobody listens at: said in words, nothing saved, the first run is not completed.
        let free = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let mut out = Vec::new();
        let typed = format!("3\n127.0.0.1:{free}\n");
        let mut input: &[u8] = typed.as_bytes();
        assert!(ask(&say(), &mut input, &mut out, &home, &home, Ui::Tui, None).is_err());
        assert!(!home.join("link").join("node-id").exists());

        // A joined machine runs a node and no hub, and its settings read back.
        let joined = Machine::joined(&home);
        assert_eq!(joined.roles, ["node"]);
        assert_eq!(Machine::parse(&joined.to_toml()), Some(joined));

        // Input ended without an answer.
        let mut input: &[u8] = b"";
        assert!(ask(
            &say(),
            &mut input,
            &mut Vec::new(),
            &home,
            &home,
            Ui::Tui,
            None
        )
        .is_err());
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
