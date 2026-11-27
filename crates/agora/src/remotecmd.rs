//! `agora remote tailscale|status|off` (ADR-0067 rung 3): reaching the hub from outside this machine, on purpose, and never without
//! the screen password.
//!
//! Three paths exist. The hub listens on loopback only (nothing outside reaches it); on a local network address; or it is reached
//! through the tailnet (its name and addresses are in the hub's certificate, and `tailscale serve` can give it a real HTTPS name) or on
//! a public address. Every path beyond the local network needs the screen password to be set first: `agora remote tailscale` refuses
//! without one, a hub started on a public address refuses without one, and `agora password clear` refuses while such a path is on.
//!
//! What Agora turned on is recorded in the hub's folder, so that `agora remote off` undoes exactly that and nothing of the person's
//! own `tailscale serve` rules:
//! - `<hub dir>/remote-names`: one certificate name per line (the tailnet name and addresses). The running hub reads it again every few
//!   seconds and, when a name is new, issues a new certificate from its own CA (docs/specs/pairing.md section 5b); it is also read at start.
//! - `<hub dir>/remote-serve`: the one `tailscale serve` rule Agora added (`port=`, `target=`, `name=`).

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::path::Path;
use std::process::ExitCode;

use agora_hub::names::checked_name;
use agora_hub::password::ScreenPassword;
use agora_hub::proxy::{self, ProxyTrust, Stored};
use agora_hub::store::SqliteStore;
use agora_i18n::{key, Arg};
use agora_node::doctor::{Check, Status};
use agora_platform::InterfaceAddr;

use crate::hubcli::{hub_dir, own_hub_addr, LISTEN_FILE};
use crate::tailscale::{self, Problem};
use crate::{Opts, Say};

/// The names Agora added to the hub's certificate (see the module text).
pub(crate) const NAMES_FILE: &str = "remote-names";
const SERVE_FILE: &str = "remote-serve";

/// The HTTPS port `tailscale serve` gets unless `--port` says another: not 443, which a person's own rule is likelier to use.
const DEFAULT_SERVE_PORT: u16 = 8443;

/// At most this many names are read from the names file (the tailnet name and its two addresses, with room for one more).
const MAX_SAVED_NAMES: usize = 4;

/// Where an address is reachable from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    /// This machine.
    Loopback,
    /// A private or link-local address: the local network.
    Lan,
    /// A Tailscale address.
    Tailnet,
    /// Anything else: the internet.
    Public,
}

pub(crate) fn class_of(ip: IpAddr) -> Class {
    match ip {
        IpAddr::V4(a) if a.is_loopback() => Class::Loopback,
        IpAddr::V4(_) if tailscale::is_tailnet_ip(ip) => Class::Tailnet,
        IpAddr::V4(a) if a.is_private() || a.is_link_local() => Class::Lan,
        IpAddr::V4(_) => Class::Public,
        IpAddr::V6(a) if a.is_loopback() => Class::Loopback,
        IpAddr::V6(a) => {
            if let Some(v4) = a.to_ipv4_mapped() {
                return class_of(IpAddr::V4(v4));
            }
            if tailscale::is_tailnet_ip(ip) {
                Class::Tailnet
            } else if a.segments()[0] & 0xfe00 == 0xfc00 || a.segments()[0] & 0xffc0 == 0xfe80 {
                Class::Lan
            } else {
                Class::Public
            }
        }
    }
}

/// Which paths reach a listener.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Exposure {
    pub loopback: bool,
    pub lan: bool,
    pub tailnet: bool,
    pub public: bool,
}

/// What a listen address reaches, given the machine's interfaces: a specific address reaches what it is; a wildcard reaches every
/// address of every interface that is up (`0.0.0.0` the IPv4 ones, `::` both).
pub(crate) fn exposure_of(listen: IpAddr, interfaces: &[InterfaceAddr]) -> Exposure {
    let mut e = Exposure::default();
    let mut add = |ip: IpAddr| match class_of(ip) {
        Class::Loopback => e.loopback = true,
        Class::Lan => e.lan = true,
        Class::Tailnet => e.tailnet = true,
        Class::Public => e.public = true,
    };
    if listen.is_unspecified() {
        add(IpAddr::from([127, 0, 0, 1]));
        for i in interfaces
            .iter()
            .filter(|i| i.up && !i.loopback && !i.transient)
        {
            if i.ip.is_ipv4() || listen.is_ipv6() {
                add(i.ip);
            }
        }
    } else {
        add(listen);
    }
    e
}

/// [`exposure_of`] with this machine's interfaces. When the system will not list them, a wildcard is taken to be public: the check
/// that matters (the screen password) must not pass for a reason it could not look at.
pub(crate) fn exposure(listen: IpAddr) -> Exposure {
    exposure_with(listen, agora_platform::interface_addresses())
}

fn exposure_with(listen: IpAddr, interfaces: std::io::Result<Vec<InterfaceAddr>>) -> Exposure {
    match interfaces {
        Ok(v) => exposure_of(listen, &v),
        Err(_) if listen.is_unspecified() => Exposure {
            loopback: true,
            public: true,
            ..Exposure::default()
        },
        Err(_) => exposure_of(listen, &[]),
    }
}

/// The most bytes read from the names file (four names fit many times over).
const MAX_NAMES_FILE: u64 = 4096;

/// The certificate names in the names file: a regular file only (not a link to a device or a pipe), read up to [`MAX_NAMES_FILE`]
/// bytes, every line checked again as a tailnet name or address, at most [`MAX_SAVED_NAMES`], no name twice. A line that does not
/// pass is skipped, so a damaged or hand-edited file never stops the hub or reaches a certificate.
pub(crate) fn saved_names(dir: &Path) -> Vec<String> {
    use std::io::Read;
    let path = dir.join(NAMES_FILE);
    let regular = std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_file());
    if !regular {
        return Vec::new();
    }
    let mut text = Vec::new();
    let read =
        std::fs::File::open(&path).and_then(|f| f.take(MAX_NAMES_FILE).read_to_end(&mut text));
    if read.is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&text);
    let mut out: Vec<String> = Vec::new();
    for line in text.lines().take(64) {
        if let Some(n) = tailscale::saved_name(line) {
            if !out.contains(&n) && out.len() < MAX_SAVED_NAMES {
                out.push(n);
            }
        }
    }
    out
}

/// The `tailscale serve` rule Agora added.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ServeRecord {
    port: u16,
    target: String,
    name: String,
}

fn read_serve(dir: &Path) -> Option<ServeRecord> {
    let text = std::fs::read_to_string(dir.join(SERVE_FILE)).ok()?;
    let value = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(k)?.strip_prefix('='))
            .map(str::to_string)
    };
    Some(ServeRecord {
        port: value("port")?.parse().ok().filter(|p| *p != 0)?,
        target: value("target")?,
        name: checked_name(&value("name")?)?,
    })
}

fn write_serve(dir: &Path, r: &ServeRecord) -> std::io::Result<()> {
    let text = format!("port={}\ntarget={}\nname={}\n", r.port, r.target, r.name);
    agora_platform::write_private(&dir.join(SERVE_FILE), text.as_bytes())
}

/// Whether a screen password is set. A hub folder with no database has none (and none is made by asking).
pub(crate) fn password_set(dir: &Path) -> bool {
    let db = dir.join("hub.db");
    db.is_file()
        && SqliteStore::open(&db)
            .map(|s| ScreenPassword::new(&s).is_set())
            .unwrap_or(false)
}

/// What the running hub listens on, from `<hub dir>/listen`.
fn listening(dir: &Path) -> Option<SocketAddr> {
    std::fs::read_to_string(dir.join(LISTEN_FILE))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// A refusal: a stable code and its sentence, on standard error.
fn refuse(say: &Say, code: &str, sentence: &'static str, args: &[(&str, Arg)]) -> ExitCode {
    let message = say.text(sentence, args);
    say.err(
        key!("cli-error"),
        &[("code", code.into()), ("message", message.into())],
    );
    ExitCode::from(1)
}

fn password_required(say: &Say) -> ExitCode {
    refuse(
        say,
        "remote.password_required",
        key!("remote-password_required"),
        &[],
    )
}

/// The rule for a hub that listens on `listen`: on a public address the screen password must be set first. Run before the hub binds
/// (and by the front door before it starts the hub), so a hub is never open to the internet without it.
pub(crate) fn check_listen(say: &Say, dir: &Path, listen: &str) -> Result<(), ExitCode> {
    let Ok(addrs) = listen.to_socket_addrs() else {
        // Not an address: binding says why.
        return Ok(());
    };
    if addrs.map(|a| exposure(a.ip())).any(|e| e.public) && !password_set(dir) {
        return Err(password_required(say));
    }
    Ok(())
}

fn on(b: bool) -> Arg {
    (if b { "yes" } else { "no" }).into()
}

fn problem_refusal(say: &Say, p: Problem) -> ExitCode {
    let reason: Arg = p.word().into();
    match p {
        Problem::NotInstalled => refuse(
            say,
            "remote.tailscale_missing",
            key!("remote-tailscale_missing"),
            &[],
        ),
        Problem::LoggedOut => refuse(
            say,
            "remote.tailscale_logged_out",
            key!("remote-tailscale_logged_out"),
            &[],
        ),
        Problem::Timeout => refuse(
            say,
            "remote.tailscale_timeout",
            key!("remote-tailscale_timeout"),
            &[],
        ),
        Problem::NotOperator => refuse(
            say,
            "remote.tailscale_not_operator",
            key!("remote-tailscale_not_operator"),
            &[("user", tailscale::this_user().into())],
        ),
        _ => refuse(
            say,
            "remote.tailscale_unreadable",
            key!("remote-tailscale_unreadable"),
            &[("reason", reason)],
        ),
    }
}

fn url(name: &str, port: u16) -> String {
    if port == 443 {
        format!("https://{name}/")
    } else {
        format!("https://{name}:{port}/")
    }
}

pub fn remote(say: &Say, o: &Opts, op: &str) -> ExitCode {
    let dir = hub_dir(o);
    match op {
        "tailscale" => tailscale_on(say, o, &dir),
        "proxy" => proxy_on(say, o, &dir),
        "status" => status(say, &dir),
        _ => off(say, &dir),
    }
}

/// The proxy file, read without following a link and without waiting on a FIFO (the platform opener).
pub(crate) fn read_proxy(dir: &Path) -> Stored {
    proxy::read_from(&dir.join(proxy::FILE), agora_platform::open_read_no_follow)
}

/// A name the person typed, made safe to print in a sentence: no control characters, at most 80 of them.
fn shown(text: &str) -> Arg {
    text.chars()
        .filter(|c| !c.is_control())
        .take(80)
        .collect::<String>()
        .into()
}

/// `agora remote proxy --trust-header NAME --host PUBLIC.NAME [--host ...] [--proxy-addr IP ...]` (ADR-0067 rung 4): the hub trusts one
/// reverse proxy (a tunnel provider's connector or any proxy the owner runs) for the real client address, and answers only the names
/// given. Off until this is run; `agora remote off` takes it back. The screen password comes first, as for every way in.
fn proxy_on(say: &Say, o: &Opts, dir: &Path) -> ExitCode {
    let Some(header) = o.trust_header.as_deref().filter(|_| !o.hosts.is_empty()) else {
        say.err(key!("cli-bad-arguments"), &[]);
        return ExitCode::from(2);
    };
    if !password_set(dir) {
        return password_required(say);
    }
    if !proxy::valid_header(header) {
        return refuse(
            say,
            "remote.proxy_header_invalid",
            key!("remote-proxy_header_invalid"),
            &[("header", shown(header))],
        );
    }
    let mut hosts = Vec::new();
    for h in &o.hosts {
        match proxy::checked_host(h) {
            Some(n) if hosts.len() < proxy::MAX_HOSTS => hosts.push(n),
            _ => {
                return refuse(
                    say,
                    "remote.proxy_host_invalid",
                    key!("remote-proxy_host_invalid"),
                    &[("host", shown(h))],
                )
            }
        }
    }
    let mut addrs = Vec::new();
    for a in &o.proxy_addrs {
        match a.parse::<IpAddr>() {
            Ok(ip)
                if addrs.len() < proxy::MAX_PROXIES
                    && !proxy::canon(ip).is_unspecified()
                    && !proxy::canon(ip).is_multicast() =>
            {
                addrs.push(ip)
            }
            _ => {
                return refuse(
                    say,
                    "remote.proxy_addr_invalid",
                    key!("remote-proxy_addr_invalid"),
                    &[("addr", shown(a))],
                )
            }
        }
    }
    let Some(trust) = ProxyTrust::new(header, &hosts, &addrs) else {
        return refuse(
            say,
            "remote.proxy_host_invalid",
            key!("remote-proxy_host_invalid"),
            &[("host", shown(&o.hosts.join(" ")))],
        );
    };
    let written = std::fs::create_dir_all(dir).and_then(|()| {
        agora_platform::write_private(&dir.join(proxy::FILE), trust.render().as_bytes())
    });
    if let Err(e) = written {
        say.err(key!("cli-hub-failed"), &[("reason", e.to_string().into())]);
        return ExitCode::from(1);
    }
    say.out(
        key!("remote-proxy-on"),
        &[
            ("hosts", trust.hosts().join(", ").into()),
            ("header", trust.header().into()),
        ],
    );
    say.out(key!("remote-proxy-access-hint"), &[]);
    if listening(dir).is_some_and(|l| {
        let e = exposure(l.ip());
        e.lan || e.tailnet || e.public
    }) {
        say.out(key!("remote-proxy-listen-hint"), &[]);
    }
    say.out(key!("remote-password-reminder"), &[]);
    ExitCode::SUCCESS
}

fn tailscale_on(say: &Say, o: &Opts, dir: &Path) -> ExitCode {
    if o.port == Some(0) {
        say.err(key!("cli-bad-arguments"), &[]);
        return ExitCode::from(2);
    }
    // First of all: nothing reaches the hub from outside without the screen password, and nothing is changed before that is so.
    if !password_set(dir) {
        return password_required(say);
    }
    // The rule must point at the port the hub really bound, never at a guess.
    let local = if o.serve {
        match own_hub_addr(dir) {
            Some(a) => a,
            None => {
                return refuse(
                    say,
                    "remote.hub_not_running",
                    key!("remote-hub_not_running"),
                    &[("dir", dir.display().to_string().into())],
                )
            }
        }
    } else {
        String::new()
    };
    let net = match tailscale::status() {
        Ok(n) => n,
        Err(p) => return problem_refusal(say, p),
    };
    let fail = |e: std::io::Error| {
        say.err(key!("cli-hub-failed"), &[("reason", e.to_string().into())]);
        ExitCode::from(1)
    };
    // The serve rule is checked before anything is written: a port that is someone else's is left alone.
    let serve = if o.serve {
        let port = o.port.unwrap_or(DEFAULT_SERVE_PORT);
        let target = format!("https+insecure://{local}");
        let state = match tailscale::serve_port(port) {
            Ok(s) => s,
            Err(p) => return problem_refusal(say, p),
        };
        // Agora's own rule (recorded, and still exactly as recorded) is updated, also when the hub has moved to another port since.
        let ours = read_serve(dir).is_some_and(|r| {
            r.port == port && state.only_proxy.as_deref() == Some(r.target.as_str())
        });
        if state.used && !ours {
            return refuse(
                say,
                "remote.serve_port_in_use",
                key!("remote-serve_port_in_use"),
                &[("port", i64::from(port).into())],
            );
        }
        Some((port, target))
    } else {
        None
    };
    if let Err(e) = std::fs::create_dir_all(dir) {
        return fail(e);
    }
    let names = net.names();
    if let Err(e) = agora_platform::write_private(
        &dir.join(NAMES_FILE),
        format!("{}\n", names.join("\n")).as_bytes(),
    ) {
        return fail(e);
    }
    say.out(
        key!("remote-names-added"),
        &[("names", names.join(", ").into())],
    );
    if let Some((port, target)) = serve {
        if let Err(p) = tailscale::serve_on(port, &target) {
            // The names stay (harmless: a name is not a way in); `remote off` takes them out.
            return problem_refusal(say, p);
        }
        let record = ServeRecord {
            port,
            target,
            name: net.dns_name.clone(),
        };
        if let Err(e) = write_serve(dir, &record) {
            return fail(e);
        }
        say.out(
            key!("remote-serve-on"),
            &[("url", url(&net.dns_name, port).into())],
        );
    } else if listening(dir).is_some_and(|l| {
        let e = exposure(l.ip());
        !e.tailnet && !e.public
    }) {
        say.out(key!("remote-tailscale-loopback-hint"), &[]);
    }
    say.out(key!("remote-password-reminder"), &[]);
    ExitCode::SUCCESS
}

/// What Agora knows about how this hub is reached.
struct Reach {
    /// Tailscale Funnel is on for something of this machine (asked only when Agora has a tailnet record).
    funnel: bool,
    listen: Option<SocketAddr>,
    exposure: Exposure,
    names: Vec<String>,
    serve: Option<ServeRecord>,
    password: bool,
    /// ADR-0067 rung 4: a trusted reverse proxy is on (or its file is damaged).
    proxy: Stored,
}

impl Reach {
    fn read(dir: &Path) -> Self {
        let listen = listening(dir);
        let (names, serve) = (saved_names(dir), read_serve(dir));
        let funnel =
            (serve.is_some() || !names.is_empty()) && tailscale::funnel_on().unwrap_or(false);
        Reach {
            funnel,
            listen,
            exposure: listen.map(|l| exposure(l.ip())).unwrap_or_default(),
            names,
            serve,
            password: password_set(dir),
            proxy: read_proxy(dir),
        }
    }

    fn proxy_on(&self) -> bool {
        matches!(self.proxy, Stored::On(_))
    }

    /// The hub listens on more than this machine, so the proxy is not the only way to reach it.
    fn listens_beyond_loopback(&self) -> bool {
        self.listen.is_some_and(|l| {
            let e = exposure(l.ip());
            e.lan || e.tailnet || e.public
        })
    }

    fn tailnet(&self) -> bool {
        self.exposure.tailnet || self.serve.is_some() || !self.names.is_empty()
    }

    /// A path from outside this machine's own network is on (the hub listens on a tailnet or public address, or Agora set one up).
    fn remote(&self) -> bool {
        self.tailnet() || self.public() || self.proxy_on()
    }

    /// Open to the whole internet: a public listen address, or Tailscale Funnel.
    fn public(&self) -> bool {
        self.exposure.public || self.funnel
    }

    /// A way in that Agora or the owner turned on on purpose: a recorded name or serve rule, or a hub bound to a public address. A hub
    /// that only happens to listen on every address of a machine that has a tailnet interface is a warning, not one of these.
    fn explicit(&self) -> bool {
        self.serve.is_some() || !self.names.is_empty() || self.public() || self.proxy_on()
    }
}

fn status(say: &Say, dir: &Path) -> ExitCode {
    let r = Reach::read(dir);
    match r.listen {
        Some(l) => say.out(
            key!("remote-status-listen"),
            &[("addr", l.to_string().into())],
        ),
        None => say.out(
            key!("remote-status-not-running"),
            &[("dir", dir.display().to_string().into())],
        ),
    }
    say.out(key!("remote-status-this-machine"), &[]);
    say.out(key!("remote-status-lan"), &[("on", on(r.exposure.lan))]);
    say.out(key!("remote-status-tailnet"), &[("on", on(r.tailnet()))]);
    if !r.names.is_empty() {
        say.out(
            key!("remote-status-names"),
            &[("names", r.names.join(", ").into())],
        );
    }
    if let Some(s) = &r.serve {
        say.out(
            key!("remote-status-serve"),
            &[("url", url(&s.name, s.port).into())],
        );
    }
    if r.funnel {
        say.out(key!("remote-status-public-funnel"), &[]);
    } else {
        say.out(
            key!("remote-status-public"),
            &[("on", on(r.exposure.public))],
        );
    }
    say.out(
        if r.password {
            key!("remote-status-password-set")
        } else {
            key!("remote-status-password-not-set")
        },
        &[],
    );
    match &r.proxy {
        Stored::On(t) => {
            say.out(
                key!("remote-status-proxy"),
                &[
                    ("hosts", t.hosts().join(", ").into()),
                    ("header", t.header().into()),
                ],
            );
            say.out(key!("remote-proxy-warning"), &[]);
        }
        Stored::Damaged => say.err(key!("remote-status-proxy-damaged"), &[]),
        Stored::Off => {}
    }
    if r.remote() && !r.password {
        say.err(key!("remote-status-no-password"), &[]);
    }
    if r.public() {
        let cause = if r.funnel && !r.exposure.public {
            "funnel"
        } else {
            "listen"
        };
        say.out(key!("remote-public-warning"), &[("cause", cause.into())]);
    }
    ExitCode::SUCCESS
}

fn off(say: &Say, dir: &Path) -> ExitCode {
    let mut did = false;
    let mut code = ExitCode::SUCCESS;
    if std::fs::symlink_metadata(dir.join(NAMES_FILE)).is_ok() {
        did = true;
        match std::fs::remove_file(dir.join(NAMES_FILE)) {
            Ok(()) => say.out(key!("remote-off-names"), &[]),
            Err(e) => {
                say.err(key!("cli-hub-failed"), &[("reason", e.to_string().into())]);
                code = ExitCode::from(1);
            }
        }
    }
    if let Some(rec) = read_serve(dir) {
        did = true;
        match tailscale::serve_port(rec.port) {
            // Gone already (the person removed it): nothing to undo.
            Ok(s) if !s.used => {
                let _ = std::fs::remove_file(dir.join(SERVE_FILE));
                say.out(key!("remote-off-serve-gone"), &[]);
            }
            // The one rule Agora added, still as it was: remove that rule and no other.
            Ok(s) if s.only_proxy.as_deref() == Some(rec.target.as_str()) => {
                match tailscale::serve_off(rec.port) {
                    Ok(()) => {
                        let _ = std::fs::remove_file(dir.join(SERVE_FILE));
                        say.out(
                            key!("remote-off-serve"),
                            &[("port", i64::from(rec.port).into())],
                        );
                    }
                    Err(p) => {
                        problem_refusal(say, p);
                        code = ExitCode::from(1);
                    }
                }
            }
            // Someone changed the rule on that port: it is not Agora's any more, so it stays exactly as it is.
            Ok(_) => {
                let _ = std::fs::remove_file(dir.join(SERVE_FILE));
                say.out(
                    key!("remote-off-serve-foreign"),
                    &[("port", i64::from(rec.port).into())],
                );
            }
            Err(p) => {
                // The record stays, so the next `remote off` can finish.
                problem_refusal(say, p);
                code = ExitCode::from(1);
            }
        }
    } else if std::fs::symlink_metadata(dir.join(SERVE_FILE)).is_ok() {
        did = true;
        match std::fs::remove_file(dir.join(SERVE_FILE)) {
            // The record could not be read, so which port Agora served is unknown: the rule may still be there.
            Ok(()) => say.err(key!("remote-off-serve-damaged"), &[]),
            Err(e) => {
                say.err(key!("cli-hub-failed"), &[("reason", e.to_string().into())]);
                code = ExitCode::from(1);
            }
        }
    }
    if std::fs::symlink_metadata(dir.join(proxy::FILE)).is_ok() {
        did = true;
        match std::fs::remove_file(dir.join(proxy::FILE)) {
            Ok(()) => say.out(key!("remote-off-proxy"), &[]),
            Err(e) => {
                say.err(key!("cli-hub-failed"), &[("reason", e.to_string().into())]);
                code = ExitCode::from(1);
            }
        }
    }
    if !did {
        say.out(key!("remote-off-nothing"), &[]);
    }
    code
}

/// The checks of `agora doctor` for remote access. A machine that never used any of it gets none (and `tailscale` is not run).
pub(crate) fn checks(o: &Opts) -> Vec<Check> {
    let dir = hub_dir(o);
    let r = Reach::read(&dir);
    let mut out = Vec::new();
    let check = |id: &'static str, status: Status, args: Vec<(&'static str, String)>| Check {
        id,
        status,
        detail: String::new(),
        args,
    };
    if r.remote() {
        // A remote path without the password is never fine; a hub that only happens to listen on a tailnet address is a warning.
        out.push(match (r.password, r.explicit()) {
            (true, _) => check("remote-password-set", Status::Ok, vec![]),
            (false, true) => check("remote-password-missing", Status::Fail, vec![]),
            (false, false) => check("remote-password-missing-tailnet", Status::Warn, vec![]),
        });
    }
    if r.exposure.public {
        out.push(check("remote-public-listen", Status::Warn, vec![]));
    }
    match &r.proxy {
        Stored::On(t) => {
            // Every time: the hub is reachable from the internet through the proxy, and the proxy is what decides who gets that far.
            out.push(check(
                "remote-proxy-on",
                Status::Warn,
                vec![
                    ("hosts", t.hosts().join(", ")),
                    ("header", t.header().to_string()),
                ],
            ));
            if let (true, Some(l)) = (r.listens_beyond_loopback(), r.listen) {
                out.push(check(
                    "remote-proxy-listen",
                    Status::Warn,
                    vec![("addr", l.to_string())],
                ));
            }
        }
        Stored::Damaged => out.push(check("remote-proxy-damaged", Status::Warn, vec![])),
        Stored::Off => {}
    }
    if r.funnel {
        out.push(check("remote-funnel", Status::Warn, vec![]));
    }
    if r.serve.is_some() || !r.names.is_empty() {
        match tailscale::status() {
            Ok(net) => out.push(check(
                "remote-tailnet",
                Status::Ok,
                vec![("name", net.dns_name)],
            )),
            Err(p) => out.push(check(
                "remote-tailnet-unreachable",
                Status::Warn,
                vec![("reason", p.word().to_string())],
            )),
        }
        let have = std::fs::read_to_string(dir.join("server-cert.pem"))
            .ok()
            .and_then(|pem| agora_hub::ca::cert_names(&pem).ok());
        if let Some(have) = have {
            let missing: Vec<String> = r
                .names
                .iter()
                .filter(|n| !have.contains(n))
                .cloned()
                .collect();
            if !missing.is_empty() {
                out.push(check(
                    "remote-names-missing",
                    Status::Warn,
                    vec![("names", missing.join(", "))],
                ));
            }
        }
    }
    out
}

/// `agora password clear` while a way in that was turned on on purpose is on: the password is what protects it, so it stays until that
/// is off. A hub that merely listens on every address of a machine that has a tailnet interface is warned about (`status`, `doctor`)
/// and does not stop the owner from clearing: nothing Agora set up depends on the password there.
pub(crate) fn clear_refused(say: &Say, dir: &Path) -> Option<ExitCode> {
    let r = Reach::read(dir);
    if !r.explicit() {
        return None;
    }
    // Which way out depends on the cause: Agora's own records are undone by `remote off`; a listen address only by restarting the hub.
    let recorded = r.serve.is_some() || !r.names.is_empty() || r.proxy_on();
    Some(if recorded {
        refuse(
            say,
            "remote.password_in_use",
            key!("remote-password_in_use"),
            &[],
        )
    } else {
        refuse(
            say,
            "remote.password_in_use",
            key!("remote-password_in_use_listen"),
            &[],
        )
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn iface(s: &str) -> InterfaceAddr {
        InterfaceAddr {
            name: "x".into(),
            ip: ip(s),
            prefix_len: 24,
            up: true,
            loopback: false,
            transient: false,
        }
    }

    #[test]
    fn addresses_are_classed_by_who_can_reach_them() {
        for (a, c) in [
            ("127.0.0.1", Class::Loopback),
            ("::1", Class::Loopback),
            ("::ffff:127.0.0.1", Class::Loopback),
            ("192.168.1.20", Class::Lan),
            ("10.1.2.3", Class::Lan),
            ("172.20.0.1", Class::Lan),
            ("169.254.1.1", Class::Lan),
            ("fe80::1", Class::Lan),
            ("fd12::1", Class::Lan),
            ("100.64.0.1", Class::Tailnet),
            ("100.127.1.1", Class::Tailnet),
            ("fd7a:115c:a1e0::5", Class::Tailnet),
            ("100.128.0.1", Class::Public),
            ("203.0.113.7", Class::Public),
            ("172.32.0.1", Class::Public),
            ("2001:db8::1", Class::Public),
            ("0.0.0.0", Class::Public),
        ] {
            assert_eq!(class_of(ip(a)), c, "{a}");
        }
    }

    #[test]
    fn a_wildcard_reaches_what_the_machines_interfaces_are() {
        let vps = [
            iface("203.0.113.7"),
            iface("100.64.0.9"),
            iface("2001:db8::7"),
        ];
        let e = exposure_of(ip("0.0.0.0"), &vps);
        assert_eq!(
            (e.loopback, e.lan, e.tailnet, e.public),
            (true, false, true, true)
        );
        let laptop = [iface("192.168.1.20")];
        let e = exposure_of(ip("0.0.0.0"), &laptop);
        assert_eq!((e.lan, e.tailnet, e.public), (true, false, false));
        // `0.0.0.0` is IPv4 only: an IPv6 public address is reached by `::` alone.
        let v6 = [iface("2001:db8::7")];
        assert!(!exposure_of(ip("0.0.0.0"), &v6).public);
        assert!(exposure_of(ip("::"), &v6).public);
        let e = exposure_of(ip("127.0.0.1"), &vps);
        assert_eq!(
            (e.loopback, e.lan, e.tailnet, e.public),
            (true, false, false, false)
        );
        assert!(exposure_of(ip("203.0.113.7"), &[]).public);
    }

    #[test]
    fn a_machine_that_will_not_list_its_interfaces_is_taken_to_be_public_for_a_wildcard() {
        let refused = || Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        for wildcard in ["0.0.0.0", "::"] {
            let e = exposure_with(ip(wildcard), refused());
            assert!(e.public, "{wildcard}: fails closed");
        }
        // A specific address needs no listing: it is what it is.
        let e = exposure_with(ip("127.0.0.1"), refused());
        assert_eq!((e.loopback, e.public), (true, false));
        let e = exposure_with(ip("192.168.1.5"), refused());
        assert_eq!((e.lan, e.public), (true, false));
        assert!(exposure_with(ip("203.0.113.7"), refused()).public);
    }

    #[test]
    fn the_names_file_gives_only_plain_names() {
        let d = std::env::temp_dir().join(format!("agora-remote-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join(NAMES_FILE),
            "Box.tail1.ts.net\n*.ts.net\nbo\u{202e}x.ts.net\n100.64.0.7\nbox.tail1.ts.net\n\nfd7a:115c:a1e0::7\na.ts.net..\nexample.com\n8.8.8.8\nc.tail1.ts.net\ne.tail1.ts.net\n",
        )
        .unwrap();
        assert_eq!(
            saved_names(&d),
            [
                "box.tail1.ts.net",
                "100.64.0.7",
                "fd7a:115c:a1e0::7",
                "c.tail1.ts.net"
            ]
        );
        std::fs::write(d.join(NAMES_FILE), [0xff, 0xfe, 0x00]).unwrap();
        assert!(saved_names(&d).is_empty());
        // Only the first 4 KiB is read: a name after a huge first line is never seen, and the file is never read whole.
        let big = format!("{}\nbox.tail1.ts.net\n", "a".repeat(10_000));
        std::fs::write(d.join(NAMES_FILE), big).unwrap();
        assert!(saved_names(&d).is_empty());
        // A link (here to a folder, elsewhere to a device or a pipe) is not a names file.
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(d.join(NAMES_FILE));
            std::fs::write(d.join("real"), "box.tail1.ts.net\n").unwrap();
            std::os::unix::fs::symlink(d.join("real"), d.join(NAMES_FILE)).unwrap();
            assert!(saved_names(&d).is_empty());
        }
    }

    #[test]
    fn a_serve_record_round_trips_and_a_damaged_one_is_nothing() {
        let d = std::env::temp_dir().join(format!("agora-remote-serve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let r = ServeRecord {
            port: 8443,
            target: "https+insecure://127.0.0.1:7443".into(),
            name: "box.tail1.ts.net".into(),
        };
        write_serve(&d, &r).unwrap();
        assert_eq!(read_serve(&d), Some(r));
        std::fs::write(d.join(SERVE_FILE), "port=0\ntarget=x\nname=a.ts.net\n").unwrap();
        assert_eq!(read_serve(&d), None);
        std::fs::write(d.join(SERVE_FILE), "port=1\ntarget=x\nname=*.ts.net\n").unwrap();
        assert_eq!(read_serve(&d), None);
    }
}
