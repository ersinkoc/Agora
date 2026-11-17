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

/// Runs `tailscale ARGS` and returns what it printed on standard output (limited). The program's own error text is never kept.
fn run(args: &[&str], wait: Duration) -> Result<String, Problem> {
    let mut cmd = Command::new("tailscale");
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    agora_platform::no_console_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Problem::NotInstalled
        } else {
            Problem::Failed
        }
    })?;
    let mut out = child.stdout.take().ok_or(Problem::Failed)?;
    // Set by the reader when the program printed more than the limit.
    let too_big = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = std::sync::Arc::clone(&too_big);
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut out).take(MAX_OUTPUT as u64 + 1).read_to_end(&mut buf);
        flag.store(buf.len() > MAX_OUTPUT, std::sync::atomic::Ordering::Release);
        buf
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {}
            Err(_) => break None,
        }
        let big = too_big.load(std::sync::atomic::Ordering::Acquire);
        if big || started.elapsed() > wait {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(if big {
                Problem::TooBig
            } else {
                Problem::Timeout
            });
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let buf = reader.join().unwrap_or_default();
    if buf.len() > MAX_OUTPUT {
        return Err(Problem::TooBig);
    }
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
}

/// Removes the rule on HTTPS port `port` (and only that one).
pub fn serve_off(port: u16) -> Result<(), Problem> {
    run(&["serve", &format!("--https={port}"), "off"], SERVE_WAIT).map(|_| ())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

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
