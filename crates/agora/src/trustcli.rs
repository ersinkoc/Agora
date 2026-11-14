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

/// Is this device the hub owner's own (a session without a person: the owner's)? Only those are offered by default.
fn owners(d: &DeviceInfo) -> bool {
    d.user_id.is_empty() || d.user_id == "owner"
}

/// How a listed device stands against what this machine trusts already.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Standing {
    /// Not trusted here under that id: a new device.
    New,
    /// The id is trusted here with another key: trusting this one would replace it.
    Replaces,
}

/// The registered devices worth asking about: not revoked, well formed, registered by the holder of the key (proof of possession,
/// verified here), and not already trusted here with that key without an end. Devices whose proof does not verify are returned apart.
fn candidates(
    file: &std::path::Path,
    devices: &[DeviceInfo],
) -> (Vec<(DeviceInfo, Standing)>, Vec<DeviceInfo>) {
    let (trusted, _) = device_trust::read_entries(file);
    let mut out = Vec::new();
    let mut unproven = Vec::new();
    for d in devices {
        if d.revoked_ms != 0
            || !device_trust::valid_device_id(&d.device_id)
            || !device_trust::valid_key(&d.public_key)
        {
            continue;
        }
        if !device_trust::registration_verifies(d) {
            unproven.push(d.clone());
            continue;
        }
        match trusted.iter().find(|t| t.id == d.device_id) {
            Some(t) if t.key == d.public_key && t.expires_ms.is_none() => {}
            Some(t) if t.key != d.public_key => out.push((d.clone(), Standing::Replaces)),
            _ => out.push((d.clone(), Standing::New)),
        }
    }
    (out, unproven)
}

/// Is `answer` a yes? `words` are the locale's yes words (`y, yes`).
pub(crate) fn is_yes(answer: &str, words: &str) -> bool {
    let a = answer.trim().to_lowercase();
    !a.is_empty() && words.split(',').any(|w| w.trim().to_lowercase() == a)
}

/// Longest name or person shown for a device the hub lists.
const SHOWN: usize = 40;

/// One device as the owner sees it before deciding: its name and person cleaned of anything that could move, hide or reorder text
/// (they come from the hub), when it was registered, and on a line of its own the fingerprint this machine computed from the key.
fn device_lines(say: &Say, d: &DeviceInfo, standing: Standing) -> Vec<String> {
    let mut lines = vec![
        say.text(
            if owners(d) {
                key!("cli-devices-line")
            } else {
                key!("cli-devices-line-user")
            },
            &[
                ("name", device_trust::shown(&d.name, SHOWN).into()),
                ("id", d.device_id.clone().into()),
                ("when", crate::hubcli::utc(d.created_ms).into()),
                ("user", device_trust::shown(&d.user_id, SHOWN).into()),
            ],
        ),
        say.text(
            key!("cli-devices-fingerprint"),
            &[("fingerprint", fingerprint(&d.public_key).into())],
        ),
    ];
    if standing == Standing::Replaces {
        lines.push(say.text(
            key!("cli-devices-replaces"),
            &[("id", d.device_id.clone().into())],
        ));
    }
    // Fluent wraps arguments in isolation marks; nothing invisible reaches the terminal.
    lines
        .into_iter()
        .map(|l| device_trust::shown(&l, usize::MAX))
        .collect()
}

fn show(say: &Say, d: &DeviceInfo, standing: Standing) {
    use std::io::Write;
    let mut out = std::io::stdout();
    for l in device_lines(say, d, standing) {
        let _ = writeln!(out, "{l}");
    }
}

/// Shows the devices the hub lists and trusts the ones the owner says yes to (with `--yes`, exactly the ones whose whole fingerprints
/// were given). Used by `agora node trust --from-hub` and after `agora node enroll`. Only the hub owner's own devices are offered; other
/// people's are listed apart and trusted only with `--include-others` and their fingerprints. A device whose id this machine trusts
/// with another key is trusted only with `--replace`.
pub fn offer(say: &Say, o: &Opts, devices: &[DeviceInfo]) -> ExitCode {
    if o.yes != !o.fingerprints.is_empty() || (o.include_others && !o.yes) {
        // `--yes` names what it covers, `--fingerprint` only means something with it, and other people's devices need both.
        return bad(say);
    }
    let file = trust_file(o);
    if devices.is_empty() {
        say.out(key!("cli-devices-none"), &[]);
        return ExitCode::SUCCESS;
    }
    let (list, unproven) = candidates(&file, devices);
    for d in &unproven {
        say.err(
            key!("cli-devices-unproven"),
            &[("id", d.device_id.clone().into())],
        );
    }
    let (mine, others): (Vec<_>, Vec<_>) = list.into_iter().partition(|(d, _)| owners(d));
    if mine.is_empty() && others.is_empty() {
        say.out(key!("cli-devices-none-new"), &[]);
        return ExitCode::SUCCESS;
    }
    if !mine.is_empty() {
        say.out(
            key!("cli-devices-offer"),
            &[("count", (mine.len() as i64).into())],
        );
        for (d, standing) in &mine {
            show(say, d, *standing);
        }
    }
    if !others.is_empty() {
        say.out(
            key!("cli-devices-others"),
            &[("count", (others.len() as i64).into())],
        );
        for (d, standing) in &others {
            show(say, d, *standing);
        }
    }
    let mut code = ExitCode::SUCCESS;
    let allowed = |(_, standing): &&(DeviceInfo, Standing)| o.replace || *standing == Standing::New;
    let chosen: Vec<&(DeviceInfo, Standing)> = if o.yes {
        let pool: Vec<&(DeviceInfo, Standing)> = mine
            .iter()
            .chain(others.iter().filter(|_| o.include_others))
            .collect();
        let mut picked = Vec::new();
        for given in &o.fingerprints {
            let want = normalize_fingerprint(given);
            match pool.iter().find(|(d, _)| {
                want.len() == 24 && normalize_fingerprint(&fingerprint(&d.public_key)) == want
            }) {
                Some(p) => picked.push(*p),
                None => {
                    say.err(
                        key!("cli-devices-unmatched"),
                        &[("fingerprint", device_trust::shown(given, 64).into())],
                    );
                    code = ExitCode::from(1);
                }
            }
        }
        picked
    } else if mine.is_empty() {
        // Other people's devices are never trusted by a yes on the terminal.
        say.out(key!("cli-devices-others-need-flag"), &[]);
        return ExitCode::SUCCESS;
    } else if std::io::stdin().is_terminal() {
        let mut out = std::io::stdout();
        let _ = write!(out, "{} ", say.text(key!("cli-devices-question"), &[]));
        let _ = out.flush();
        let mut answer = String::new();
        let _ = std::io::stdin().lock().read_line(&mut answer);
        if is_yes(&answer, &say.text(key!("cli-devices-yes"), &[])) {
            mine.iter().collect()
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
    for p in chosen {
        let d = &p.0;
        if !allowed(&p) {
            say.err(
                key!("cli-devices-replace-refused"),
                &[("id", d.device_id.clone().into())],
            );
            code = ExitCode::from(1);
            continue;
        }
        match device_trust::confirm(&file, &d.device_id, &d.public_key) {
            Ok(()) => say.out(
                key!("cli-devices-trusted"),
                &[
                    ("name", device_trust::shown(&d.name, SHOWN).into()),
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
#[allow(clippy::unwrap_used, clippy::expect_used)]
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

    fn signed(id: &str, name: &str, user: &str) -> (DeviceInfo, Vec<u8>) {
        use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
        let rng = ring::rand::SystemRandom::new();
        let p = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let k =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, p.as_ref(), &rng).unwrap();
        let key = k.public_key().as_ref().to_vec();
        let sig = k
            .sign(&rng, &device_trust::register_message(id, name, &key, 5))
            .unwrap()
            .as_ref()
            .to_vec();
        (
            DeviceInfo {
                device_id: id.into(),
                name: name.into(),
                public_key: key.clone(),
                user_id: user.into(),
                signed_at_ms: 5,
                registration_signature: sig,
                ..Default::default()
            },
            key,
        )
    }

    #[test]
    fn a_name_from_the_hub_cannot_hide_or_fake_the_fingerprint() {
        // Review finding 1: escape sequences, a carriage return and bidi controls in the name.
        let say = Say(agora_i18n::Catalog::for_locale("en"));
        let evil = "Phone\u{1b}[2K\r\u{1b}[1A    fingerprint 0000-0000-0000-0000-0000-0000\u{202e}\u{200b}";
        let (d, key) = signed("web-1", evil, "owner");
        let lines = device_lines(&say, &d, Standing::New);
        for l in &lines {
            assert!(!l.chars().any(char::is_control), "{l:?}");
            assert_eq!(
                &device_trust::shown(l, usize::MAX),
                l,
                "nothing hidden left: {l:?}"
            );
        }
        assert!(
            lines[0].chars().count() < 120,
            "the name is capped: {}",
            lines[0]
        );
        assert_eq!(
            lines[1].trim(),
            format!("fingerprint {}", fingerprint(&key)),
            "computed here, on its own line"
        );
        assert_eq!(fingerprint(&key).len(), 29, "96 bits");
    }

    #[test]
    fn only_devices_with_their_own_proof_are_offered_and_a_key_swap_is_flagged() {
        let dir = std::env::temp_dir().join(format!("agora-cand-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("trusted-devices");
        let (good, _) = signed("web-1", "Firefox", "owner");
        let (mut forged, _) = signed("web-2", "Chrome", "owner");
        forged.public_key = signed("x", "x", "owner").1;
        let (swap, _) = signed("web-3", "Edge", "owner");
        let (other_key_holder, other_key) = signed("web-3", "Edge", "owner");
        drop(other_key_holder);
        device_trust::confirm(&file, "web-3", &other_key).unwrap();
        let (list, unproven) = candidates(&file, &[good.clone(), forged, swap]);
        assert_eq!(
            unproven
                .iter()
                .map(|d| d.device_id.as_str())
                .collect::<Vec<_>>(),
            ["web-2"]
        );
        let standings: Vec<(&str, Standing)> = list
            .iter()
            .map(|(d, s)| (d.device_id.as_str(), *s))
            .collect();
        assert_eq!(
            standings,
            [("web-1", Standing::New), ("web-3", Standing::Replaces)]
        );
        assert!(owners(&good));
        assert!(!owners(&signed("web-4", "x", "eda").0));
        let _ = std::fs::remove_dir_all(dir);
    }
}
