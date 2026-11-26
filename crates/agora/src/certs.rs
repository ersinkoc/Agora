//! Certificate lifetime checks for `agora doctor` (and the machine list). A certificate lasts 90 days and renews itself when a third
//! of its life is left (docs/adr/0085-machine-and-hub-certificates-renew-themselves.md): the days left are shown, and so is what the
//! hub said when this machine's renewal was refused. The files are read as they are on disk.

use std::path::Path;

use agora_hub::ca::{cert_not_after_secs, expiry, Expiry};
use agora_node::doctor::{Check, Status};

fn secs_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// The detail of a check whose file is there but cannot be read as a certificate.
pub const UNUSABLE: &str = "unusable";

/// One check for the certificate in `file`, with id `id`; nothing when the file is not there (the machine has no such role).
fn one(id: &'static str, file: &Path, now: i64) -> Option<Check> {
    let end = match std::fs::read_to_string(file) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        // A file that is there but cannot be read as text (binary, no permission) is as unusable as one that is not a certificate.
        Err(_) => None,
        Ok(pem) => cert_not_after_secs(&pem).ok(),
    };
    let (status, days) = match end.map(|end| expiry(end, now)) {
        Some(Expiry::Fine(d)) => (Status::Ok, d),
        Some(Expiry::Soon(d)) => (Status::Warn, d),
        Some(Expiry::Expired(d)) => (Status::Fail, d),
        // A file that cannot be read, or not as a certificate, fails with its own sentence: it has not ended, it is unusable
        // (Codex review of F851: it was told as ended, with the advice to join again).
        None => {
            return Some(Check {
                id,
                status: Status::Fail,
                detail: UNUSABLE.to_string(),
                args: vec![("file", file.display().to_string())],
            })
        }
    };
    Some(Check {
        id,
        status,
        detail: days.to_string(),
        args: vec![("file", file.display().to_string())],
    })
}

/// The node's own certificate (`<link dir>/cert.pem`, written by enrollment) and the hub's server certificate
/// (`<hub dir>/server-cert.pem`, written each time the hub starts).
pub fn checks(link_dir: &Path, hub_dir: &Path, now: i64) -> Vec<Check> {
    let mut out: Vec<Check> = [
        one("cert-node", &link_dir.join("cert.pem"), now),
        one("cert-hub", &hub_dir.join("server-cert.pem"), now),
    ]
    .into_iter()
    .flatten()
    .collect();
    out.extend(renewal(link_dir, now));
    out
}

/// What the machine's own renewal says (ADR-0085), from the files the node keeps beside the certificate: a refusal that only a new join
/// can answer, a certificate that is no longer the newest (another holder may exist), an answer that did not check out, or a request
/// still waiting for the hub's answer. Nothing when it works. A refusal recorded for another certificate than the one in use is ignored.
fn renewal(link_dir: &Path, now: i64) -> Option<Check> {
    use agora_node::link_renewal::{pending_exists, state};
    let pem = std::fs::read_to_string(link_dir.join("cert.pem")).ok()?;
    let ends = cert_not_after_secs(&pem).ok()?;
    let file = link_dir.display().to_string();
    // `asked` says whether the time of the last try is known; `when` is that time as a date.
    let made = |id: &'static str, status, detail: String, asked_at: i64| Check {
        id,
        status,
        detail,
        args: vec![
            ("file", file.clone()),
            ("asked", if asked_at > 0 { "yes" } else { "no" }.to_string()),
            (
                "when",
                crate::hubcli::utc(u64::try_from(asked_at).unwrap_or(0).saturating_mul(1000)),
            ),
        ],
    };
    match state(link_dir, ends) {
        Some(s) if s.code == "join_again" => Some(made(
            "cert-renew-join-again",
            Status::Fail,
            s.detail,
            s.asked_at,
        )),
        Some(s) if s.code == "superseded" => Some(made(
            "cert-renew-superseded",
            Status::Warn,
            s.detail,
            s.asked_at,
        )),
        // Rejected for time (ADR-0085 Amendment A): the request is kept, and this machine's clock is the likely cause.
        Some(s) if s.code == "rejected" && s.detail == "validity" => {
            Some(made("cert-renew-clock", Status::Warn, s.detail, s.asked_at))
        }
        Some(s) if s.code == "rejected" => Some(made(
            "cert-renew-rejected",
            Status::Warn,
            // `certificate`, remembered by a version before the split, is `chain`.
            if s.detail == "certificate" {
                "chain".to_string()
            } else {
                s.detail
            },
            s.asked_at,
        )),
        // The hub this machine is connected to is older than renewal: nothing can be asked of it yet. Only a warning when the end is
        // near.
        Some(s) if s.code == "hub_old" => Some(made(
            "cert-renew-hub-old",
            if ends - now < agora_hub::ca::EXPIRY_WARN_DAYS * 86_400 {
                Status::Warn
            } else {
                Status::Ok
            },
            String::new(),
            s.asked_at,
        )),
        // A request is waiting for the hub's answer (the hub was away, or asked to wait): the machine keeps working with the
        // certificate it has until the end.
        _ if pending_exists(link_dir) && ends > now => {
            Some(made("cert-renew-pending", Status::Ok, String::new(), 0))
        }
        _ => None,
    }
}

pub fn checks_now(link_dir: &Path, hub_dir: &Path) -> Vec<Check> {
    checks(link_dir, hub_dir, secs_now())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_hub::ca::{new_device_request, Ca};

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("agora-certs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_machine_without_certificates_gets_no_lines() {
        let d = dir("none");
        assert!(checks(&d, &d, secs_now()).is_empty());
    }

    #[test]
    fn a_fresh_certificate_is_ok_one_near_its_end_warns_and_an_expired_one_fails() {
        let d = dir("states");
        let (_, csr) = new_device_request().unwrap();
        let ca = Ca::generate("ca").unwrap();
        let pem = ca.issue_node("n", &csr).unwrap().cert_pem;
        std::fs::write(d.join("cert.pem"), &pem).unwrap();
        let end = cert_not_after_secs(&pem).unwrap();
        let now = end - 100 * 86_400;
        let c = checks(&d, &d, now);
        assert_eq!((c[0].id, c[0].status), ("cert-node", Status::Ok));
        assert_eq!(c[0].detail, "100");
        let c = checks(&d, &d, end - 10 * 86_400 - 5);
        assert_eq!((c[0].status, c[0].detail.as_str()), (Status::Warn, "10"));
        let c = checks(&d, &d, end + 3 * 86_400 + 5);
        assert_eq!((c[0].status, c[0].detail.as_str()), (Status::Fail, "3"));
    }

    #[test]
    fn the_hub_server_certificate_is_checked_when_the_hub_wrote_one() {
        let d = dir("hub");
        let ca = Ca::generate("ca").unwrap();
        let (cert, _) = ca.issue_server(&["localhost".to_string()]).unwrap();
        std::fs::write(d.join("server-cert.pem"), &cert).unwrap();
        let end = cert_not_after_secs(&cert).unwrap();
        let c = checks(&d, &d, end + 1);
        assert_eq!((c[0].id, c[0].status), ("cert-hub", Status::Fail));
        assert_eq!(c[0].detail, "0");
        std::fs::write(d.join("server-cert.pem"), "not a certificate").unwrap();
        let c = checks(&d, &d, 0);
        assert_eq!(
            (c[0].status, c[0].detail.as_str()),
            (Status::Fail, UNUSABLE)
        );
        // F851: a file that is there but cannot be read as text (a DER certificate, binary garbage) fails too, as unusable, not
        // as ended.
        std::fs::write(d.join("server-cert.pem"), [0x30u8, 0x82, 0xff, 0xfe]).unwrap();
        let c = checks(&d, &d, 0);
        assert_eq!(
            (c[0].status, c[0].detail.as_str()),
            (Status::Fail, UNUSABLE)
        );
    }

    fn renewal_dir(name: &str) -> (std::path::PathBuf, i64) {
        let d = dir(name);
        let (_, csr) = new_device_request().unwrap();
        let ca = Ca::generate("ca").unwrap();
        let pem = ca.issue_node("n", &csr).unwrap().cert_pem;
        std::fs::write(d.join("cert.pem"), &pem).unwrap();
        (d, cert_not_after_secs(&pem).unwrap())
    }

    fn state(d: &Path, code: &str, detail: &str, ends: i64) {
        std::fs::write(
            d.join("renew-state"),
            format!("{code}\n{detail}\n{ends}\n0\n1700000000\n0\n"),
        )
        .unwrap();
    }

    fn renewals(d: &Path, now: i64) -> Vec<(&'static str, Status, String)> {
        checks(d, d, now)
            .into_iter()
            .filter(|c| c.id.starts_with("cert-renew"))
            .map(|c| (c.id, c.status, c.detail))
            .collect()
    }

    #[test]
    fn a_machine_that_renews_fine_says_nothing_about_renewal() {
        let (d, _) = renewal_dir("renew-fine");
        assert!(renewals(&d, secs_now()).is_empty());
    }

    #[test]
    fn a_refusal_only_a_new_join_answers_is_a_problem_and_one_for_another_certificate_is_ignored() {
        let (d, ends) = renewal_dir("renew-join");
        state(&d, "join_again", "revoked", ends);
        assert_eq!(
            renewals(&d, secs_now()),
            [("cert-renew-join-again", Status::Fail, "revoked".to_string())]
        );
        // The machine joined again since: the state was about the certificate it had before.
        state(&d, "join_again", "revoked", ends + 1);
        assert!(renewals(&d, secs_now()).is_empty());
    }

    #[test]
    fn a_certificate_that_is_no_longer_the_newest_is_a_warning_and_a_rejected_answer_too() {
        let (d, ends) = renewal_dir("renew-sup");
        state(&d, "superseded", "superseded", ends);
        assert_eq!(renewals(&d, secs_now())[0].0, "cert-renew-superseded");
        assert_eq!(renewals(&d, secs_now())[0].1, Status::Warn);
        state(&d, "rejected", "key", ends);
        assert_eq!(
            renewals(&d, secs_now()),
            [("cert-renew-rejected", Status::Warn, "key".to_string())]
        );

        state(&d, "rejected", "validity", ends);
        assert_eq!(
            renewals(&d, secs_now()),
            [("cert-renew-clock", Status::Warn, "validity".to_string())]
        );
        state(&d, "rejected", "certificate", ends);
        assert_eq!(
            renewals(&d, secs_now()),
            [("cert-renew-rejected", Status::Warn, "chain".to_string())]
        ); // Waiting out the hub's hour is not a problem to report.
        state(&d, "too_frequent", "too_frequent", ends);
        assert!(renewals(&d, secs_now()).is_empty());
    }

    #[test]
    fn a_renewal_that_waits_for_the_hubs_answer_is_shown_until_the_certificate_ends() {
        let (d, ends) = renewal_dir("renew-pending");
        std::fs::write(d.join("renew-pending.pem"), "x").unwrap();
        assert_eq!(
            renewals(&d, secs_now()),
            [("cert-renew-pending", Status::Ok, String::new())]
        );
        assert!(renewals(&d, ends + 5).is_empty());
    }

    #[test]
    fn the_last_try_and_an_old_hub_are_told_and_a_damaged_state_file_says_nothing() {
        let (d, ends) = renewal_dir("renew-when");
        state(&d, "join_again", "unknown", ends);
        let c = checks(&d, &d, secs_now())
            .into_iter()
            .find(|c| c.id == "cert-renew-join-again")
            .unwrap();
        assert!(c.args.contains(&("asked", "yes".to_string())));
        assert!(c
            .args
            .contains(&("when", "2023-11-14 22:13 UTC".to_string())));
        // An older hub: nothing to ask of it, an informative line (a warning once the end is near).
        state(&d, "hub_old", "", ends);
        assert_eq!(
            renewals(&d, secs_now()),
            [("cert-renew-hub-old", Status::Ok, String::new())]
        );
        assert_eq!(renewals(&d, ends - 86_400)[0].1, Status::Warn);
        // A state file that cannot be read is no state, and no line.
        std::fs::write(d.join("renew-state"), [0xffu8, 0xfe, 0x00]).unwrap();
        assert!(renewals(&d, secs_now()).is_empty());
        std::fs::write(d.join("renew-state"), "join_again\n").unwrap();
        assert!(renewals(&d, secs_now()).is_empty());
    }
}
