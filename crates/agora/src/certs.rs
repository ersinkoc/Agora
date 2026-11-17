//! Certificate lifetime checks for `agora doctor` (and the machine list). A machine certificate lasts 90 days and nothing renews
//! it yet (docs/adr/0085-machine-and-hub-certificates-renew-themselves.md), so the owner is told in time. The files are read as they are on disk.

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
    [
        one("cert-node", &link_dir.join("cert.pem"), now),
        one("cert-hub", &hub_dir.join("server-cert.pem"), now),
    ]
    .into_iter()
    .flatten()
    .collect()
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
}
