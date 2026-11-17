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

use crate::hubcli::{link_dir, store_link};
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

/// Explains, in the person's words, how to find an address to join (discovery by itself comes later).
pub(crate) fn explain(say: &Say) {
    say.out(key!("join-help"), &[]);
}

/// `agora join [ADDRESS] [--id NAME] [--replace]`.
pub fn join(say: &Say, o: &Opts, address: Option<&str>) -> ExitCode {
    let Some(address) = address else {
        explain(say);
        return ExitCode::from(2);
    };
    match run(
        say,
        address,
        o.id.as_deref(),
        o.replace,
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
            let url = format!("wss://{dial}");
            if let Err(e) = store_link(&url, &id, &v.ca_pem, &v.cert_pem, &key_pem) {
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
