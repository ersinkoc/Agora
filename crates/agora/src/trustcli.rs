//! Which devices this machine trusts, decided at the machine (ADR-0020, ADR-0061).
//!
//! - `agora node trust DEVICE_ID PUBLIC_KEY_HEX`: as it always was.
//! - `agora node trust --from-hub [--yes --fingerprint FP]...`: asks the hub for the registered devices on a connection of its own (the
//!   node's certificate), shows each as *name · fingerprint · registered when*, and asks on this terminal whether to trust them.
//! - `agora node enroll ...` asks the same question about the devices the hub sent with the certificate.
//! - `agora node untrust DEVICE_ID`: stops trusting a device here, at once, and remembers it as revoked.
//!
//! The hub's list is only a list of candidates: nothing here trusts a device without the owner's yes on this terminal, or `--yes`
//! with that device's fingerprint on the command line. A terminal that is not interactive is a no.

use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use agora_i18n::key;
use agora_node::approvals::TRUSTED_DEVICES_FILE;
use agora_node::device_trust::{self, fingerprint, normalize_fingerprint};
use agora_protocol::v1::DeviceInfo;

use crate::{home, Opts, Say};

fn trust_file(o: &Opts) -> PathBuf {
    o.data
        .clone()
        .unwrap_or_else(|| home().join("data"))
        .join(TRUSTED_DEVICES_FILE)
}

/// `agora node trust ...`
pub fn node_trust(say: &Say, o: &Opts) -> ExitCode {
    if o.from_hub {
        if !o.positional.is_empty() {
            return bad(say);
        }
        return from_hub(say, o);
    }
    let (Some(id), Some(key), None) = (
        o.positional.first(),
        o.positional.get(1),
        o.positional.get(2),
    ) else {
        return bad(say);
    };
    if o.yes || !o.fingerprints.is_empty() {
        return bad(say);
    }
    let file = trust_file(o);
    let result = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .map_err(|e| e.to_string())
        .and_then(|()| agora_node::approvals::trust_device(&file, id, key));
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

/// `agora node untrust DEVICE_ID`
pub fn node_untrust(say: &Say, o: &Opts) -> ExitCode {
    let (Some(id), None) = (o.positional.first(), o.positional.get(1)) else {
        return bad(say);
    };
    let file = trust_file(o);
    let _ = file.parent().map(std::fs::create_dir_all);
    match device_trust::untrust(&file, id, agora_node::grants::now_ms()) {
        Ok(true) => {
            say.out(key!("cli-untrusted"), &[("id", id.clone().into())]);
            ExitCode::SUCCESS
        }
        Ok(false) => {
            say.out(key!("cli-untrust-unknown"), &[("id", id.clone().into())]);
            ExitCode::SUCCESS
        }
        Err(e) => {
            say.err(key!("cli-trust-failed"), &[("reason", e.into())]);
            ExitCode::from(1)
        }
    }
}

fn bad(say: &Say) -> ExitCode {
    say.err(key!("cli-bad-arguments"), &[]);
    say.err(key!("cli-usage"), &[]);
    ExitCode::from(2)
}

fn from_hub(say: &Say, o: &Opts) -> ExitCode {
    let Some((_, link)) = crate::hubcli::load_link(crate::socket_path(o)) else {
        say.err(key!("cli-devices-not-enrolled"), &[]);
        return ExitCode::from(1);
    };
    match agora_node::uplink_client::list_devices(&link) {
        Ok(devices) => offer(say, o, &devices),
        Err(e) => {
            let reason = match e {
                agora_node::uplink_client::LinkError::Refused(code) => code,
                e => format!("{e:?}"),
            };
            say.err(key!("cli-devices-failed"), &[("reason", reason.into())]);
            ExitCode::from(1)
        }
    }
}

/// The registered devices this machine does not trust yet with that key: the ones worth asking about.
fn candidates(file: &std::path::Path, devices: &[DeviceInfo]) -> Vec<DeviceInfo> {
    let (trusted, _) = device_trust::read_entries(file);
    devices
        .iter()
        .filter(|d| {
            d.revoked_ms == 0
                && device_trust::valid_device_id(&d.device_id)
                && device_trust::valid_key(&d.public_key)
                && !trusted
                    .iter()
                    .any(|t| t.id == d.device_id && t.key == d.public_key && t.expires_ms.is_none())
        })
        .cloned()
        .collect()
}

/// Is `answer` a yes? `words` are the locale's yes words (`y, yes`).
pub(crate) fn is_yes(answer: &str, words: &str) -> bool {
    let a = answer.trim().to_lowercase();
    !a.is_empty() && words.split(',').any(|w| w.trim().to_lowercase() == a)
}

/// Shows the devices the hub lists and trusts the ones the owner says yes to (all of them, or with `--yes`, exactly the ones whose
/// fingerprints were given). Used by `agora node trust --from-hub` and after `agora node enroll`.
pub fn offer(say: &Say, o: &Opts, devices: &[DeviceInfo]) -> ExitCode {
    if o.yes != !o.fingerprints.is_empty() {
        // `--yes` names what it covers, and `--fingerprint` only means something with it.
        return bad(say);
    }
    let file = trust_file(o);
    if devices.is_empty() {
        say.out(key!("cli-devices-none"), &[]);
        return ExitCode::SUCCESS;
    }
    let list = candidates(&file, devices);
    if list.is_empty() {
        say.out(key!("cli-devices-none-new"), &[]);
        return ExitCode::SUCCESS;
    }
    say.out(
        key!("cli-devices-offer"),
        &[("count", (list.len() as i64).into())],
    );
    for d in &list {
        let line = if d.user_id.is_empty() || d.user_id == "owner" {
            key!("cli-devices-line")
        } else {
            key!("cli-devices-line-user")
        };
        say.out(
            line,
            &[
                ("name", d.name.clone().into()),
                ("fingerprint", fingerprint(&d.public_key).into()),
                ("when", crate::hubcli::utc(d.created_ms).into()),
                ("user", d.user_id.clone().into()),
            ],
        );
    }
    let mut code = ExitCode::SUCCESS;
    let chosen: Vec<&DeviceInfo> = if o.yes {
        let wanted: Vec<String> = o
            .fingerprints
            .iter()
            .map(|f| normalize_fingerprint(f))
            .collect();
        for (given, w) in o.fingerprints.iter().zip(&wanted) {
            if !list
                .iter()
                .any(|d| normalize_fingerprint(&fingerprint(&d.public_key)) == *w)
            {
                say.err(
                    key!("cli-devices-unmatched"),
                    &[("fingerprint", given.clone().into())],
                );
                code = ExitCode::from(1);
            }
        }
        list.iter()
            .filter(|d| wanted.contains(&normalize_fingerprint(&fingerprint(&d.public_key))))
            .collect()
    } else if std::io::stdin().is_terminal() {
        let mut out = std::io::stdout();
        let _ = write!(out, "{} ", say.text(key!("cli-devices-question"), &[]));
        let _ = out.flush();
        let mut answer = String::new();
        let _ = std::io::stdin().lock().read_line(&mut answer);
        if is_yes(&answer, &say.text(key!("cli-devices-yes"), &[])) {
            list.iter().collect()
        } else {
            say.out(key!("cli-devices-declined"), &[]);
            return ExitCode::SUCCESS;
        }
    } else {
        say.out(key!("cli-devices-not-asked"), &[]);
        return ExitCode::SUCCESS;
    };
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    for d in chosen {
        match device_trust::confirm(&file, &d.device_id, &d.public_key) {
            Ok(()) => say.out(
                key!("cli-devices-trusted"),
                &[
                    ("name", d.name.clone().into()),
                    ("id", d.device_id.clone().into()),
                    ("fingerprint", fingerprint(&d.public_key).into()),
                ],
            ),
            Err(e) => {
                say.err(key!("cli-trust-failed"), &[("reason", e.into())]);
                code = ExitCode::from(1);
            }
        }
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_yes_word_is_a_yes() {
        let words = "y, yes, e, evet";
        for a in ["y\n", "YES", " evet ", "e"] {
            assert!(is_yes(a, words), "{a}");
        }
        for a in ["", "\n", "n", "no", "yess", "hayır", "y yes"] {
            assert!(!is_yes(a, words), "{a}");
        }
    }
}
