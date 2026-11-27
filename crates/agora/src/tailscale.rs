//! Talks to the `tailscale` command line (ADR-0067 rung 3): which name and addresses this machine has on the tailnet, and the
//! `tailscale serve` rules. No API key, no network call of its own: it runs the program the person installed and reads what it prints.
//!
//! Everything the program prints is untrusted text. A name or address is used only after it passes a strict check (a MagicDNS name
//! under `.ts.net`, an address in the tailnet's own ranges); anything else is refused, never cleaned up. Output is read up to a limit
//! and the program is stopped when it takes too long, so a broken or hostile `tailscale` cannot hold or fill this process.

use std::io::Read;
use std::net::IpAddr;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use agora_hub::names::checked_name;

/// The most output of `tailscale` read (a tailnet of thousands of machines is a few megabytes).
const MAX_OUTPUT: usize = 4 * 1024 * 1024;

/// How long `tailscale status` may take.
const STATUS_WAIT: Duration = Duration::from_secs(15);

/// How long `tailscale serve` may take: a tailnet that has not allowed HTTPS yet makes it wait for the person in a browser.
const SERVE_WAIT: Duration = Duration::from_secs(30);

/// At most this many tailnet addresses are kept (one IPv4 and one IPv6 are what a machine has).
const MAX_IPS: usize = 2;

/// Why the program's answer could not be used. `word` is a stable word for the sentence and the log, never the program's own text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// There is no `tailscale` on the path.
    NotInstalled,
    /// It could not be started, or ended with an error.
    Failed,
    /// It did not end in time (it was stopped).
    Timeout,
    /// It printed more than [`MAX_OUTPUT`].
    TooBig,
    /// What it printed is not the status JSON.
    Garbage,
    /// This machine is not signed in to a tailnet, or Tailscale is stopped.
    LoggedOut,
    /// The status has no usable machine name (MagicDNS is off).
    NoName,
    /// The status has a machine name that is not a plain MagicDNS name; it is never used.
    BadName,
    /// `tailscale serve` was refused: only root or the user Tailscale names as its operator may change it
    /// (`sudo tailscale set --operator=$USER` once).
    NotOperator,
}

impl Problem {
    pub fn word(self) -> &'static str {
        match self {
            Problem::NotInstalled => "not_installed",
            Problem::Failed => "failed",
            Problem::Timeout => "timeout",
            Problem::TooBig => "too_big",
            Problem::Garbage => "garbage",
            Problem::LoggedOut => "logged_out",
            Problem::NoName => "no_name",
            Problem::BadName => "bad_name",
            Problem::NotOperator => "not_operator",
        }
    }
}

/// This machine on the tailnet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tailnet {
    /// The MagicDNS name, lower case, without the dot at the end: `box.tail1234.ts.net`.
    pub dns_name: String,
    /// Its tailnet addresses (at most [`MAX_IPS`]), IPv4 first.
    pub ips: Vec<IpAddr>,
}

impl Tailnet {
    /// The names for the hub's certificate: the name, then the addresses.
    pub fn names(&self) -> Vec<String> {
        std::iter::once(self.dns_name.clone())
            .chain(self.ips.iter().map(IpAddr::to_string))
            .collect()
    }
}

/// True for an address Tailscale hands out: `100.64.0.0/10` (carrier-grade NAT space) and `fd7a:115c:a1e0::/48`.
pub fn is_tailnet_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => a.octets()[0] == 100 && (a.octets()[1] & 0xc0) == 64,
        IpAddr::V6(a) => a.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
    }
}

/// A MagicDNS machine name: `machine.tailnet.ts.net`, plain letters, digits and hyphens.
fn magic_dns_name(raw: &str) -> Option<String> {
    let name = checked_name(raw)?;
    let labels: Vec<&str> = name.split('.').collect();
    (labels.len() >= 3
        && labels.last() == Some(&"net")
        && labels[labels.len() - 2] == "ts"
        && name.parse::<IpAddr>().is_err())
    .then_some(name)
}

/// A name or address from the names file, checked again on the way back in: a MagicDNS name under `.ts.net` or an address in the
/// tailnet's ranges, and nothing else (the file is the owner's, but a damaged or edited one never reaches a certificate).
pub fn saved_name(line: &str) -> Option<String> {
    let line = line.trim();
    match line.parse::<IpAddr>() {
        Ok(ip) => is_tailnet_ip(ip).then(|| ip.to_string()),
        Err(_) => magic_dns_name(line),
    }
}

/// Reads `tailscale status --json`. Never panics, whatever the text is.
pub fn parse_status(text: &str) -> Result<Tailnet, Problem> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|_| Problem::Garbage)?;
    let state = v.get("BackendState").and_then(|s| s.as_str());
    let me = v.get("Self").filter(|s| s.is_object());
    let (Some(state), Some(me)) = (state, me) else {
        return Err(Problem::Garbage);
    };
    if state != "Running" {
        return Err(Problem::LoggedOut);
    }
    let raw = me
        .get("DNSName")
        .and_then(|n| n.as_str())
        .filter(|n| !n.is_empty())
        .ok_or(Problem::NoName)?;
    let dns_name = magic_dns_name(raw).ok_or(Problem::BadName)?;
    let mut ips: Vec<IpAddr> = me
        .get("TailscaleIPs")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str()?.parse::<IpAddr>().ok())
        .filter(|ip| is_tailnet_ip(*ip))
        .collect();
    ips.sort_by_key(|ip| (ip.is_ipv6(), *ip));
    ips.dedup();
    ips.truncate(MAX_IPS);
    Ok(Tailnet { dns_name, ips })
}

/// What `tailscale serve status --json` says about one HTTPS port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServePort {
    /// Some rule listens on the port.
    pub used: bool,
    /// The port serves exactly one thing, at `/`, and it is a proxy to this target.
    pub only_proxy: Option<String>,
}

/// Reads `tailscale serve status --json` for `port`. `{}` (nothing served) is fine; anything that is not an object is garbage.
pub fn parse_serve(text: &str, port: u16) -> Result<ServePort, Problem> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|_| Problem::Garbage)?;
    let Some(top) = v.as_object() else {
        return Err(Problem::Garbage);
    };
    let key = port.to_string();
    let suffix = format!(":{port}");
    let in_tcp = top
        .get("TCP")
        .and_then(|t| t.as_object())
        .is_some_and(|t| t.contains_key(&key));
    let webs: Vec<_> = top
        .get("Web")
        .and_then(|w| w.as_object())
        .into_iter()
        .flatten()
        .filter(|(host, _)| host.ends_with(&suffix))
        .map(|(_, w)| w)
        .collect();
    let only_proxy = match webs.as_slice() {
        [one] => one
            .get("Handlers")
            .and_then(|h| h.as_object())
            .filter(|h| h.len() == 1)
            .and_then(|h| h.get("/"))
            .filter(|h| h.as_object().is_some_and(|o| o.len() == 1))
            .and_then(|h| h.get("Proxy"))
            .and_then(|p| p.as_str())
            .map(str::to_string),
        _ => None,
    };
    Ok(ServePort {
        used: in_tcp || !webs.is_empty(),
        only_proxy,
    })
}

/// True when `tailscale serve status --json` says anything is open to the whole internet (Tailscale Funnel): `AllowFunnel` at the top
/// or in any `Foreground` configuration. Garbage is an error, not "no".
pub fn parse_funnel(text: &str) -> Result<bool, Problem> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|_| Problem::Garbage)?;
    if !v.is_object() {
        return Err(Problem::Garbage);
    }
    let open = |c: &serde_json::Value| {
        c.get("AllowFunnel")
            .and_then(|a| a.as_object())
            .is_some_and(|a| a.values().any(|x| x.as_bool() == Some(true)))
    };
    Ok(open(&v)
        || v.get("Foreground")
            .and_then(|f| f.as_object())
            .is_some_and(|f| f.values().any(open)))
}

/// Whether Tailscale Funnel is on for anything of this machine.
pub fn funnel_on() -> Result<bool, Problem> {
    parse_funnel(&run(&["serve", "status", "--json"], STATUS_WAIT)?)
}

/// Turns of the wait loop of [`run_program`] (tests count them to see that it never spins).
#[cfg(test)]
static LOOPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Runs `tailscale ARGS` and returns what it printed on standard output (limited). The program's own error text is never kept.
fn run(args: &[&str], wait: Duration) -> Result<String, Problem> {
    run_program(std::ffi::OsStr::new("tailscale"), args, wait, MAX_OUTPUT)
}

/// Runs `program` with `args`: at most `wait` in all and `max` bytes of output, whatever the program does. The output is read on a
/// thread that is never joined (a child of the program that keeps the pipe open would hold a join for as long as it lives): the
/// answer is what arrived when the program ended or the time ran out. The program and everything it started are ended when it
/// is stopped, and once it has ended.
fn run_program(
    program: &std::ffi::OsStr,
    args: &[&str],
    wait: Duration,
    max: usize,
) -> Result<String, Problem> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    agora_platform::no_console_window(&mut cmd);
    agora_platform::own_process_group(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Problem::NotInstalled
        } else {
            Problem::Failed
        }
    })?;
    let mut out = child.stdout.take().ok_or(Problem::Failed)?;
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut chunk = [0u8; 16 * 1024];
        while let Ok(n) = out.read(&mut chunk) {
            if n == 0 || tx.send(chunk[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let started = Instant::now();
    let mut buf: Vec<u8> = Vec::new();
    let take = |buf: &mut Vec<u8>, chunk: Vec<u8>| {
        buf.extend_from_slice(&chunk);
        buf.len() > max
    };
    let status = loop {
        #[cfg(test)]
        LOOPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(chunk) => {
                if take(&mut buf, chunk) {
                    agora_platform::kill_process_tree(&mut child);
                    return Err(Problem::TooBig);
                }
                continue;
            }
            // The program closed its output and keeps running: `recv_timeout` answers at once now, so wait here instead of spinning.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {}
            Err(_) => break None,
        }
        if started.elapsed() > wait {
            agora_platform::kill_process_tree(&mut child);
            return Err(Problem::Timeout);
        }
    };
    // Ended: what it wrote is already in the pipe. Take it (a moment at most), then end whatever it left behind.
    let grace = Instant::now();
    while grace.elapsed() < Duration::from_millis(200) {
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(chunk) => {
                if take(&mut buf, chunk) {
                    agora_platform::kill_process_tree(&mut child);
                    return Err(Problem::TooBig);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if grace.elapsed() > Duration::from_millis(60) {
                    break;
                }
            }
        }
    }
    agora_platform::kill_process_tree(&mut child);
    match status {
        Some(s) if s.success() => Ok(String::from_utf8_lossy(&buf).into_owned()),
        _ => Err(Problem::Failed),
    }
}

/// This machine on the tailnet, from `tailscale status --json`.
pub fn status() -> Result<Tailnet, Problem> {
    parse_status(&run(&["status", "--json"], STATUS_WAIT)?)
}

/// The state of one HTTPS port of `tailscale serve`.
pub fn serve_port(port: u16) -> Result<ServePort, Problem> {
    parse_serve(&run(&["serve", "status", "--json"], STATUS_WAIT)?, port)
}

/// Publishes `target` on HTTPS port `port` of this machine's tailnet name, in the background.
pub fn serve_on(port: u16, target: &str) -> Result<(), Problem> {
    run(
        &["serve", "--bg", &format!("--https={port}"), target],
        SERVE_WAIT,
    )
    .map(|_| ())
    .map_err(why_serve_failed)
}

/// Removes the rule on HTTPS port `port` (and only that one).
pub fn serve_off(port: u16) -> Result<(), Problem> {
    run(&["serve", &format!("--https={port}"), "off"], SERVE_WAIT)
        .map(|_| ())
        .map_err(why_serve_failed)
}

/// A `tailscale serve` that failed: [`Problem::NotOperator`] when this user is neither root nor Tailscale's operator (the usual
/// reason on Linux, where Tailscale says "Access denied: serve config denied"), else the failure as it was. Tailscale's own error
/// text is never read: its preferences say who the operator is.
fn why_serve_failed(p: Problem) -> Problem {
    if p != Problem::Failed {
        return p;
    }
    match run(&["debug", "prefs"], STATUS_WAIT) {
        Ok(prefs) if operator_missing(&prefs, &this_user()) => Problem::NotOperator,
        _ => p,
    }
}

/// The user this runs as, by name (`USER`, else `LOGNAME`); empty when neither is set.
pub fn this_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default()
}

/// Whether `user` (not root, and known) is not the operator Tailscale's preferences (`tailscale debug prefs`) name. Unreadable
/// preferences or an unknown user say nothing (false).
fn operator_missing(prefs: &str, user: &str) -> bool {
    if user.is_empty() || user == "root" {
        return false;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(prefs) else {
        return false;
    };
    match v.get("OperatorUser") {
        Some(serde_json::Value::String(op)) => op != user,
        Some(serde_json::Value::Null) | None => v.is_object(),
        Some(_) => false,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Who may change `tailscale serve`, from Tailscale's preferences (the maintainer's blackview, 2026-11-27: "Access denied:
    /// serve config denied" until `sudo tailscale set --operator=$USER`).
    #[test]
    fn a_user_who_is_not_tailscales_operator_is_told_so() {
        assert!(operator_missing(r#"{"OperatorUser":""}"#, "dev"));
        assert!(operator_missing(r#"{"OperatorUser":"someone"}"#, "dev"));
        assert!(operator_missing(r#"{"WantRunning":true}"#, "dev"));
        assert!(!operator_missing(r#"{"OperatorUser":"dev"}"#, "dev"));
        assert!(!operator_missing(r#"{"OperatorUser":""}"#, "root"));
        assert!(!operator_missing(r#"{"OperatorUser":""}"#, ""));
        assert!(!operator_missing("not json", "dev"));
        assert!(!operator_missing(r#"{"OperatorUser":7}"#, "dev"));
    }

    /// A program (a shell script) in a folder of its own.
    #[cfg(unix)]
    fn script(name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("agora-ts-run-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("prog");
        std::fs::write(&f, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        f
    }

    /// The caps hold however the program behaves: a child that keeps the output open, a program that never ends, endless output.
    #[cfg(unix)]
    #[test]
    fn the_time_and_size_caps_hold_for_a_program_that_leaves_children_or_never_ends() {
        let two = Duration::from_secs(2);
        // Prints its answer and ends, but a child of it keeps standard output open for 12 seconds.
        let p = script("child", "sleep 12 &\necho ok");
        let t = Instant::now();
        let r = run_program(p.as_os_str(), &[], two, 1024);
        assert_eq!(r.as_deref(), Ok("ok\n"));
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        // Whatever a program leaves behind is dead afterwards: on the normal path and on the timeout path.
        for (name, body, wait) in [
            (
                "killnormal",
                "sleep 60 &\necho $! > \"$0.pid\"\necho ok",
                Duration::from_secs(2),
            ),
            (
                "killtimeout",
                "sleep 60 &\necho $! > \"$0.pid\"\nsleep 100",
                Duration::from_secs(1),
            ),
        ] {
            let p = script(name, body);
            let _ = run_program(p.as_os_str(), &[], wait, 1024);
            let pid = std::fs::read_to_string(format!("{}.pid", p.display())).unwrap();
            let pid = pid.trim();
            let end = Instant::now() + Duration::from_secs(3);
            loop {
                let out = Command::new("ps")
                    .args(["-o", "stat=", "-p", pid])
                    .output()
                    .unwrap();
                let st = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if st.is_empty() || st.starts_with('Z') {
                    break;
                }
                assert!(
                    Instant::now() < end,
                    "{name}: process {pid} is still alive ({st})"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        // A program that closes its output and keeps running does not make the wait loop spin.
        // A start can fail for a moment with "text file busy" when another test forks while the
        // script is still open for writing; that is not what this case checks, so try again.
        let mut tries = 0;
        let (t, turns) = loop {
            let p = script("closed", "exec >&-\nsleep 2");
            LOOPS.store(0, std::sync::atomic::Ordering::Relaxed);
            let t = Instant::now();
            let r = run_program(p.as_os_str(), &[], Duration::from_secs(5), 1024);
            tries += 1;
            if r != Err(Problem::Failed) || t.elapsed() >= Duration::from_secs(1) || tries >= 5 {
                break (t, LOOPS.load(std::sync::atomic::Ordering::Relaxed));
            }
        };
        assert!(t.elapsed() >= Duration::from_secs(1));
        assert!(turns < 600, "{turns} turns in {:?}", t.elapsed());
        // Never ends, and prints nothing.
        let p = script("hang", "sleep 100");
        let t = Instant::now();
        assert_eq!(
            run_program(p.as_os_str(), &[], Duration::from_secs(1), 1024),
            Err(Problem::Timeout)
        );
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        // Never ends, and keeps the output open from a child as well.
        let p = script("hangchild", "sleep 100 &\nsleep 100");
        let t = Instant::now();
        assert_eq!(
            run_program(p.as_os_str(), &[], Duration::from_secs(1), 1024),
            Err(Problem::Timeout)
        );
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        // Endless output.
        let p = script("endless", "yes");
        let t = Instant::now();
        assert_eq!(
            run_program(p.as_os_str(), &[], Duration::from_secs(10), 4096),
            Err(Problem::TooBig)
        );
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        // An error exit is a failure, and a missing program is its own word.
        let p = script("bad", "exit 3");
        assert_eq!(
            run_program(p.as_os_str(), &[], two, 1024),
            Err(Problem::Failed)
        );
        assert_eq!(
            run_program(
                std::ffi::OsStr::new("/nonexistent/agora-tailscale"),
                &[],
                two,
                1024
            ),
            Err(Problem::NotInstalled)
        );
    }

    fn status_json(state: &str, dns: &str, ips: &str) -> String {
        format!(
            r#"{{"BackendState":"{state}","Self":{{"DNSName":"{dns}","TailscaleIPs":{ips}}},"Peer":{{}}}}"#
        )
    }

    #[test]
    fn a_running_machine_gives_its_name_and_tailnet_addresses() {
        let t = parse_status(&status_json(
            "Running",
            "Box.tail1234.ts.net.",
            r#"["fd7a:115c:a1e0::1","100.101.102.103"]"#,
        ))
        .unwrap();
        assert_eq!(t.dns_name, "box.tail1234.ts.net");
        assert_eq!(
            t.names(),
            [
                "box.tail1234.ts.net",
                "100.101.102.103",
                "fd7a:115c:a1e0::1"
            ]
        );
    }

    #[test]
    fn a_machine_that_is_not_signed_in_is_said_so() {
        for state in ["NeedsLogin", "Stopped", "Starting", ""] {
            assert_eq!(
                parse_status(&status_json(state, "box.tail1234.ts.net.", "[]")),
                Err(Problem::LoggedOut),
                "{state}"
            );
        }
    }

    #[test]
    fn a_status_with_no_name_or_a_hostile_one_gives_no_certificate_name() {
        assert_eq!(
            parse_status(&status_json("Running", "", "[]")),
            Err(Problem::NoName)
        );
        assert!(matches!(
            parse_status(r#"{"BackendState":"Running","Self":{}}"#),
            Err(Problem::NoName)
        ));
        for dns in [
            "*.tail1234.ts.net.",
            "box.tail1234.ts.net.evil.example.",
            "box.example.com.",
            "ts.net.",
            "100.64.0.1",
            "bo\\u202ex.tail1234.ts.net.",
            "bo\\nx.tail1234.ts.net.",
            "bo\\u0000x.tail1234.ts.net.",
            "b o.tail1234.ts.net.",
            "caf\\u00e9.tail1234.ts.net.",
            "-box.tail1234.ts.net.",
            "box..tail1234.ts.net.",
        ] {
            assert_eq!(
                parse_status(&status_json("Running", dns, "[]")),
                Err(Problem::BadName),
                "{dns}"
            );
        }
        let long = format!("{}.tail1234.ts.net.", "a".repeat(64));
        assert_eq!(
            parse_status(&status_json("Running", &long, "[]")),
            Err(Problem::BadName)
        );
        let long = format!("{}.ts.net.", "a.".repeat(140));
        assert_eq!(
            parse_status(&status_json("Running", &long, "[]")),
            Err(Problem::BadName)
        );
    }

    #[test]
    fn only_tailnet_addresses_are_kept() {
        let t = parse_status(&status_json(
            "Running",
            "box.tail1234.ts.net.",
            r#"["8.8.8.8","10.0.0.4","127.0.0.1","::1","fe80::1","100.128.0.1","100.64.0.1","100.127.255.254","not an ip",7,null,"100.100.100.100"]"#,
        ))
        .unwrap();
        assert_eq!(t.ips.len(), 2, "{:?}", t.ips);
        assert!(t.ips.iter().all(|ip| is_tailnet_ip(*ip)));
    }

    #[test]
    fn garbage_is_garbage_and_never_a_panic() {
        for text in [
            "",
            "not json",
            "null",
            "[]",
            "42",
            "\"Running\"",
            "{}",
            r#"{"BackendState":"Running"}"#,
            r#"{"BackendState":"Running","Self":"x"}"#,
            r#"{"BackendState":7,"Self":{}}"#,
            "\u{0}\u{1}\u{2}",
            &"[".repeat(10_000),
            &"{\"a\":".repeat(10_000),
        ] {
            assert!(parse_status(text).is_err(), "{text:.40}");
        }
        let _ = parse_status(&format!(
            r#"{{"BackendState":"Running","Self":{{"DNSName":"a.b.ts.net","TailscaleIPs":[{}]}}}}"#,
            "\"100.64.0.1\",".repeat(100_000) + "\"100.64.0.2\""
        ))
        .unwrap();
    }

    #[test]
    fn funnel_is_found_at_the_top_and_in_a_foreground_session() {
        assert_eq!(parse_funnel("{}"), Ok(false));
        assert_eq!(
            parse_funnel(r#"{"AllowFunnel":{"a.ts.net:443":false}}"#),
            Ok(false)
        );
        assert_eq!(
            parse_funnel(r#"{"AllowFunnel":{"a.ts.net:443":true}}"#),
            Ok(true)
        );
        assert_eq!(
            parse_funnel(r#"{"Foreground":{"s1":{"AllowFunnel":{"a.ts.net:443":true}}}}"#),
            Ok(true)
        );
        assert_eq!(parse_funnel(r#"{"Foreground":{"s1":{}}}"#), Ok(false));
        assert_eq!(parse_funnel("[]"), Err(Problem::Garbage));
        assert_eq!(parse_funnel("x"), Err(Problem::Garbage));
    }

    #[test]
    fn a_saved_name_is_a_tailnet_name_or_address_and_nothing_else() {
        assert_eq!(
            saved_name(" Box.tail1.ts.net "),
            Some("box.tail1.ts.net".into())
        );
        assert_eq!(saved_name("100.64.0.7"), Some("100.64.0.7".into()));
        for bad in [
            "a.ts.net..",
            "a.b",
            "example.com",
            "8.8.8.8",
            "10.0.0.1",
            "*.ts.net",
            "",
            "ts.net",
            "a\u{202e}.ts.net",
        ] {
            assert_eq!(saved_name(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_serve_port_is_free_or_used_and_only_one_proxy_is_recognised() {
        assert_eq!(
            parse_serve("{}", 8443).unwrap(),
            ServePort {
                used: false,
                only_proxy: None
            }
        );
        let ours = r#"{"TCP":{"8443":{"HTTPS":true}},"Web":{"box.tail1234.ts.net:8443":{"Handlers":{"/":{"Proxy":"https+insecure://127.0.0.1:7443"}}}}}"#;
        assert_eq!(
            parse_serve(ours, 8443).unwrap(),
            ServePort {
                used: true,
                only_proxy: Some("https+insecure://127.0.0.1:7443".into())
            }
        );
        assert!(!parse_serve(ours, 443).unwrap().used);
        // Two handlers on the port: not a rule Agora made.
        let two = r#"{"TCP":{"8443":{"HTTPS":true}},"Web":{"b.ts.net:8443":{"Handlers":{"/":{"Proxy":"x"},"/api":{"Proxy":"y"}}}}}"#;
        let p = parse_serve(two, 8443).unwrap();
        assert!(p.used && p.only_proxy.is_none());
        // A handler with more than a proxy (a path, a text) is not a plain proxy.
        let text = r#"{"Web":{"b.ts.net:8443":{"Handlers":{"/":{"Text":"hi"}}}}}"#;
        let p = parse_serve(text, 8443).unwrap();
        assert!(p.used && p.only_proxy.is_none());
        assert_eq!(parse_serve("[]", 8443), Err(Problem::Garbage));
        assert_eq!(parse_serve("nope", 8443), Err(Problem::Garbage));
    }
}
