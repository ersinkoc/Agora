//! `agora join [ADDRESS]` (ADR-0065, `docs/specs/pairing.md` section 5): this machine asks a hub to let it in, shows the person a number
//! to compare with the one on the hub's screen, and, once the owner approved, keeps the certificate in the link folder exactly as
//! `agora node enroll` does.
//!
//! The protocol and every check of what the hub delivers are in `agora_node::pairing_client`; this file is the person's side: the key
//! this machine generates (it never leaves it), the one line that shows the number, the sentences, and what is stored.

use std::io::Write;
use std::process::ExitCode;

use agora_hub::ca::{csr_der, new_device_request};
use agora_i18n::key;
use agora_node::pairing_client::{pair, split_address, PairError, Timeouts, Wanted};

use crate::hubcli::{hold_link, link_dir};
use crate::{Opts, Say};

/// A machine that joined: the id the hub knows it by.
pub(crate) struct Joined {
    pub id: String,
}

/// How a join ended without joining. The sentence was already printed.
pub(crate) enum Stop {
    /// The person gave nothing usable (an address that is not one, a node that is already linked).
    Usage,
    Failed,
}

impl Stop {
    pub(crate) fn code(&self) -> ExitCode {
        match self {
            Self::Usage => ExitCode::from(2),
            Self::Failed => ExitCode::from(1),
        }
    }
}

/// How long `agora join` listens for hubs that are open for new machines.
const SEARCH: std::time::Duration = std::time::Duration::from_secs(3);

/// The queries this machine sends: from each IPv4 address of each interface that is up (not loopback, not link-local), to that
/// network's broadcast address and to the limited broadcast address, on the discovery port.
fn queries() -> Vec<(std::net::Ipv4Addr, Vec<std::net::SocketAddr>)> {
    use agora_hub::discovery::{directed_broadcast, DISCOVERY_PORT};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    let Ok(list) = agora_platform::interface_addresses() else {
        return Vec::new();
    };
    let mut out: Vec<(Ipv4Addr, Vec<SocketAddr>)> = Vec::new();
    for a in list.iter().filter(|a| a.up && !a.loopback) {
        let IpAddr::V4(ip) = a.ip else { continue };
        if ip.is_link_local() || ip.is_unspecified() || out.iter().any(|(i, _)| *i == ip) {
            continue;
        }
        let mut targets = vec![SocketAddr::from((Ipv4Addr::BROADCAST, DISCOVERY_PORT))];
        if let Some(b) = directed_broadcast(ip, a.prefix_len) {
            targets.push(SocketAddr::from((b, DISCOVERY_PORT)));
        }
        out.push((ip, targets));
    }
    out
}

/// What the person decided from the list of hubs found.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    /// This one (an index into the list).
    One(usize),
    /// Nothing found, or nothing chosen.
    None,
    /// No terminal to confirm on: a hub found by discovery is never chosen for the person, not even a single one.
    NeedsTerminal,
}

/// Discovery is never silently trusted: every hub found, one or several, is confirmed by a person at a terminal who compares its
/// fingerprint with the one `agora pair open` shows on the hub. `ask(n)` shows the `n` hubs and returns what was typed (for one hub,
/// `Some` of any text only when the person confirmed it); only a number from 1 to `n` chooses.
pub(crate) fn pick(
    count: usize,
    terminal: bool,
    ask: impl FnOnce(usize) -> Option<String>,
) -> Pick {
    if count == 0 {
        return Pick::None;
    }
    if !terminal {
        return Pick::NeedsTerminal;
    }
    match ask(count).and_then(|t| t.trim().parse::<usize>().ok()) {
        Some(i) if (1..=count).contains(&i) => Pick::One(i - 1),
        _ => Pick::None,
    }
}

/// Listens for hubs that are open for new machines, has the person confirm one, and returns its address and the SHA-256 of its CA
/// (the machine will then refuse any other CA), or says why there is none.
fn find_hub(say: &Say) -> Result<(String, [u8; 32]), Stop> {
    use agora_hub::discovery::short_fingerprint;
    use std::io::IsTerminal;
    say.out(key!("join-looking"), &[]);
    let _ = std::io::stdout().flush();
    let mut random = [0u8; 16];
    let _ = getrandom::getrandom(&mut random);
    let mut found = agora_hub::discovery::find(&queries(), SEARCH, random);
    found.sort_by(|a, b| (&a.name, a.addr).cmp(&(&b.name, b.addr)));
    if found.is_empty() {
        say.err(key!("join-none-found"), &[]);
        return Err(Stop::Failed);
    }
    let terminal = std::io::stdin().is_terminal();
    let choice = pick(found.len(), terminal, |n| {
        if n == 1 {
            let f = &found[0];
            let question = say.text(
                key!("join-found-ask"),
                &[
                    ("name", f.name.as_str().into()),
                    ("address", f.addr.to_string().into()),
                    ("fingerprint", short_fingerprint(&f.ca_fingerprint).into()),
                ],
            );
            return matches!(
                crate::paircmd::ask(say, &question),
                crate::paircmd::Answer::Yes
            )
            .then(|| "1".to_string());
        }
        say.out(key!("join-found-list-head"), &[]);
        for (i, f) in found.iter().enumerate() {
            say.out(
                key!("join-found-item"),
                &[
                    ("n", ((i + 1) as i64).into()),
                    ("name", f.name.as_str().into()),
                    ("address", f.addr.to_string().into()),
                    ("fingerprint", short_fingerprint(&f.ca_fingerprint).into()),
                ],
            );
        }
        // The same terminal-only reader as the single-hub question and the owner's approval: typed-ahead input is discarded and a
        // number that arrives too fast is ignored.
        let question = say.text(key!("join-which"), &[("count", (n as i64).into())]);
        crate::paircmd::ask_line(say, &question)
    });
    match choice {
        Pick::One(i) => {
            say.out(key!("join-found-note"), &[]);
            Ok((found[i].addr.to_string(), found[i].ca_fingerprint))
        }
        Pick::NeedsTerminal => {
            say.err(key!("pair-discovery_unconfirmed"), &[]);
            Err(Stop::Usage)
        }
        Pick::None => {
            say.err(key!("join-nothing-chosen"), &[]);
            Err(Stop::Usage)
        }
    }
}

/// `agora join [ADDRESS] [--id NAME] [--replace]`. Without an address, hubs that are open for new machines are looked for on the local
/// network (docs/specs/pairing.md section 3): what is found is only an address to try, and the whole pairing exchange follows.
pub fn join(say: &Say, o: &Opts, address: Option<&str>) -> ExitCode {
    let found;
    let mut expected_ca = None;
    let address = match address {
        Some(a) => a,
        None => {
            // Nothing is searched for when this machine could not join anyway.
            if link_dir().join("node-id").exists() && !o.replace {
                say.err(
                    key!("join-already"),
                    &[("dir", link_dir().display().to_string().into())],
                );
                return ExitCode::from(2);
            }
            found = match find_hub(say) {
                Ok((a, fp)) => {
                    expected_ca = Some(fp);
                    a
                }
                Err(stop) => return stop.code(),
            };
            &found
        }
    };
    match run(
        say,
        address,
        o.id.as_deref(),
        o.replace,
        expected_ca,
        &Timeouts::default(),
    ) {
        Ok(joined) => {
            // A machine that has no settings yet gets the ones a joined machine needs, so that `agora` starts its node.
            if let Some(m) = node_only_settings(&crate::home()) {
                if crate::frontdoor::save_if_absent(&crate::home(), &m).is_err() {
                    say.err(key!("join-settings-unsaved"), &[]);
                }
            }
            say.out(key!("join-start"), &[("id", joined.id.into())]);
            ExitCode::SUCCESS
        }
        Err(stop) => stop.code(),
    }
}

/// The settings of a machine that only runs a node and joined a hub elsewhere, when the machine has none yet.
fn node_only_settings(home: &std::path::Path) -> Option<crate::frontdoor::Machine> {
    crate::frontdoor::load(home)
        .is_none()
        .then(|| crate::frontdoor::Machine::joined(home))
}

/// Pairs with the hub at `address`, shows the number, and on approval keeps the result in the link folder. Prints every sentence
/// itself (the front door calls this too).
pub(crate) fn run(
    say: &Say,
    address: &str,
    id: Option<&str>,
    replace: bool,
    expected_ca: Option<[u8; 32]>,
    timeouts: &Timeouts,
) -> Result<Joined, Stop> {
    let link = link_dir();
    if link.join("node-id").exists() && !replace {
        say.err(
            key!("join-already"),
            &[("dir", link.display().to_string().into())],
        );
        return Err(Stop::Usage);
    }
    let Some((host, dial)) = split_address(address) else {
        say.err(key!("join-bad-address"), &[]);
        return Err(Stop::Usage);
    };
    let id = id.map_or_else(
        || crate::frontdoor::node_id_from(agora_platform::hostname()),
        str::to_string,
    );
    // The key is made here and stays here: the module that talks to the hub is given the request to sign, never the key.
    let Ok((key_pem, csr_pem)) = new_device_request() else {
        say.err(key!("join-no-key"), &[]);
        return Err(Stop::Failed);
    };
    let Ok(csr) = csr_der(&csr_pem) else {
        say.err(key!("join-no-key"), &[]);
        return Err(Stop::Failed);
    };
    say.out(
        key!("join-connecting"),
        &[("address", dial.as_str().into())],
    );
    let _ = std::io::stdout().flush();
    let wanted = Wanted {
        address,
        node_name: &id,
        os: std::env::consts::OS,
        version: env!("CARGO_PKG_VERSION"),
        csr_pem: &csr_pem,
        csr_der: &csr,
        expected_ca,
    };
    // The one place the number is written: this line, on standard output, for the person in front of this screen. It is never
    // logged and never saved.
    let mut show = |n: &agora_node::pairing_client::Number| {
        let mut out = std::io::stdout();
        let _ = writeln!(
            out,
            "{}",
            say.text(key!("join-number"), &[("number", n.grouped().into())])
        );
        let _ = out.flush();
    };
    match pair(&wanted, timeouts, &mut show) {
        Ok(v) => {
            // Looked at again: the owner may have taken minutes, and this machine may have joined meanwhile (`agora` enrolling with
            // its own hub, `agora node enroll`, another join). Without `--replace` that link is kept, never overwritten (audit F890),
            // unless it is this same hub under this same name: the hub issues a taken name only by replacing it, and the certificate
            // it replaced is revoked, so keeping that link would keep one that no longer works (Codex review of F890). The look and
            // the save hold the folder's guard together (audit F1032): a save that was under way when the join came here is over
            // before the look, so its link is seen and kept.
            let held = match hold_link(&link) {
                Ok(h) => h,
                Err(e) => {
                    say.err(
                        key!("join-store-failed"),
                        &[("reason", e.to_string().into())],
                    );
                    return Err(Stop::Failed);
                }
            };
            if held.dir().join("node-id").exists()
                && !replace
                && !superseded(held.dir(), &id, &v.ca_pem)
            {
                say.err(
                    key!("join-already"),
                    &[("dir", link.display().to_string().into())],
                );
                return Err(Stop::Usage);
            }
            let url = format!("wss://{dial}");
            if let Err(e) = held.store(&url, &id, &v.ca_pem, &v.cert_pem, &key_pem) {
                say.err(
                    key!("join-store-failed"),
                    &[("reason", e.to_string().into())],
                );
                return Err(Stop::Failed);
            }
            let fingerprint = agora_desktop_tunnel::fingerprint(&v.ca_pem).unwrap_or_default();
            say.out(
                key!("join-done"),
                &[
                    ("id", id.as_str().into()),
                    ("fingerprint", fingerprint.into()),
                ],
            );
            Ok(Joined { id })
        }
        Err(e) => {
            say_failure(say, &e, &dial, &host);
            Err(Stop::Failed)
        }
    }
}

fn say_failure(say: &Say, e: &PairError, dial: &str, host: &str) {
    let address = ("address", dial.into());
    match e {
        PairError::BadAddress => say.err(key!("join-bad-address"), &[]),
        PairError::Unreachable(reason) => say.err(
            key!("join-unreachable"),
            &[address, ("reason", reason.as_str().into())],
        ),
        PairError::Tls13Required => say.err(key!("join-tls13-required"), &[address]),
        PairError::TlsFailed(reason) => say.err(
            key!("join-tls-failed"),
            &[address, ("reason", reason.as_str().into())],
        ),
        PairError::Timeout { waiting_for } if *waiting_for == "decision" => {
            say.err(key!("join-timeout-decision"), &[]);
        }
        PairError::Timeout { .. } => say.err(key!("join-timeout-answer"), &[]),
        PairError::Hub {
            code,
            state,
            seconds,
        } => {
            let mut args: Vec<(&str, agora_i18n::Arg)> = Vec::new();
            if let Some(s) = state {
                args.push(("state", s.as_str().into()));
            }
            if let Some(s) = seconds {
                args.push(("seconds", (*s).into()));
            }
            let message = say.text(&code.replace('.', "-"), &args);
            say.err(
                key!("cli-error"),
                &[("code", code.as_str().into()), ("message", message.into())],
            );
        }
        PairError::CaMismatch => say.err(key!("pair-ca_mismatch"), &[]),
        PairError::Denied => say.err(key!("join-denied"), &[]),
        PairError::Closed => say.err(key!("join-closed"), &[]),
        PairError::Unexpected => say.err(key!("join-unexpected"), &[]),
        PairError::NameUnverified { name } => {
            say.err(
                key!("pair-name_unverified"),
                &[("name", name.as_str().into())],
            );
            say.err(key!("join-name-hint"), &[("name", host.into())]);
        }
        PairError::CertificateInvalid(what) => {
            say.err(
                key!("join-certificate-invalid"),
                &[("what", (*what).into())],
            );
        }
    }
}

/// Whether the link in `link` is this machine under `id` at the hub whose CA is `ca_pem`: the one the certificate just delivered
/// supersedes. Anything unreadable is not.
fn superseded(link: &std::path::Path, id: &str, ca_pem: &str) -> bool {
    let read = |f: &str| std::fs::read_to_string(link.join(f)).ok();
    read("node-id").is_some_and(|n| n.trim() == id)
        && read("ca.pem").is_some_and(|c| c.trim() == ca_pem.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hub_found_by_discovery_is_never_chosen_without_a_person_at_a_terminal() {
        let never = |_| panic!("nobody is asked");
        assert_eq!(pick(0, true, never), Pick::None);
        // One hub or several, no terminal: nothing is chosen and nobody is asked.
        assert_eq!(pick(1, false, never), Pick::NeedsTerminal);
        assert_eq!(pick(3, false, never), Pick::NeedsTerminal);
        // One hub with a terminal: only a person's confirmation chooses it.
        assert_eq!(pick(1, true, |_| Some("1".into())), Pick::One(0));
        assert_eq!(pick(1, true, |_| None), Pick::None);
        // Several: the person's number (one-based), and anything else chooses nothing.
        assert_eq!(
            pick(3, true, |n| (n == 3).then(|| "2\n".into())),
            Pick::One(1)
        );
        for typed in ["", "\n", "0", "4", "-1", "x", "1 2", "99999999999999999999"] {
            assert_eq!(
                pick(3, true, |_| Some(typed.to_string())),
                Pick::None,
                "{typed:?}"
            );
        }
        assert_eq!(pick(3, true, |_| None), Pick::None);
    }
}
