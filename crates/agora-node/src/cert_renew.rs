//! The node's side of certificate renewal (ADR-0085): when to renew, what a renewal's persisted state looks like, and the checks a
//! certificate the hub sent back must pass before it replaces the one in use. Pure functions and plain data; no files, no sockets, no
//! clock of their own. The driver that owns the files and the timers is in the `agora` crate (it holds the link folder's guard), and
//! the connection is [`crate::uplink_client::renew`].
//!
//! Nothing here prints a private key: [`Pending`] hides it from `Debug`.

use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use crate::pairing_client::check_client_certificate;

/// How far after this machine's clock a renewed certificate may start (ADR-0085 Amendment A): a clock up to a day behind the hub's
/// still renews at once. The hub judges the certificate on its own clock at every connection. Pairing keeps "valid now".
pub const RENEW_FUTURE_START: i64 = 24 * 3600;

/// A machine asks for a new certificate when a third of the certificate's own life is left (30 of 90 days).
pub const RENEW_AT_FRACTION: i64 = 3;

/// The start and end of a PEM certificate (the first in the text), in seconds since 1970.
pub fn validity(cert_pem: &str) -> Option<(i64, i64)> {
    let der = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .next()?
        .ok()?;
    let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref()).ok()?;
    Some((
        cert.validity().not_before.timestamp(),
        cert.validity().not_after.timestamp(),
    ))
}

/// Whether `cert_pem` has a third of its life or less left at `now`: computed from the certificate's own start and end, so a
/// hub that changes the lifetime changes nothing here. `None` when the text is not a certificate. A certificate that has ended is due
/// (the caller finds out the hub refuses it).
pub fn due(cert_pem: &str, now: i64) -> Option<bool> {
    let (from, to) = validity(cert_pem)?;
    Some(to - now <= (to - from) / RENEW_AT_FRACTION)
}

/// The fresh key and request of a renewal that has been started and not yet answered. Kept in one private file until the answer
/// is stored, so the same request can be sent again after a crash or a lost answer (the hub answers a request it already answered with
/// the same certificate, ADR-0085).
#[derive(Clone, PartialEq, Eq)]
pub struct Pending {
    pub key_pem: String,
    pub csr_pem: String,
    /// The fingerprint ([`fingerprint`]) of the certificate this request was made for, the one in use when it was made. A request
    /// kept from before this machine joined again is for another certificate and is dropped, not sent.
    pub for_cert: String,
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Pending(..)")
    }
}

impl Pending {
    /// A key and its request, as text without trailing blank space (so what is written and read back is equal).
    pub fn new(key_pem: &str, csr_pem: &str) -> Self {
        Self {
            key_pem: key_pem.trim().to_string(),
            csr_pem: csr_pem.trim().to_string(),
            for_cert: String::new(),
        }
    }

    /// This request, made for the certificate `cert_pem` (the one in use).
    #[must_use]
    pub fn bound(mut self, cert_pem: &str) -> Self {
        self.for_cert = fingerprint(cert_pem);
        self
    }

    /// The text of the file: a line naming the certificate it is for, the key's PEM, then the request's PEM.
    pub fn encode(&self) -> String {
        format!(
            "for: {}\n{}\n{}\n",
            self.for_cert,
            self.key_pem.trim_end(),
            self.csr_pem.trim_end()
        )
    }

    /// Reads [`Self::encode`]; `None` unless both parts are there and the request is the key's own (it carries the key's public key).
    pub fn decode(text: &str) -> Option<Self> {
        let at = text.find("-----BEGIN CERTIFICATE REQUEST-----")?;
        let (key_pem, csr_pem) = text.split_at(at);
        let (for_cert, key_pem) = match key_pem.trim_start().strip_prefix("for:") {
            Some(rest) => rest.split_once('\n').unwrap_or(("", "")),
            None => ("", key_pem),
        };
        let mut this = Self::new(key_pem, csr_pem);
        this.for_cert = for_cert.trim().to_string();
        if this.key_pem.is_empty() || !csr_matches_key(&this.csr_pem, &this.key_pem) {
            return None;
        }
        Some(this)
    }
}

/// The SHA-256 of the first certificate's DER in `cert_pem`, in hex; empty when the text holds none.
pub fn fingerprint(cert_pem: &str) -> String {
    let Some(Ok(der)) = CertificateDer::pem_slice_iter(cert_pem.as_bytes()).next() else {
        return String::new();
    };
    ring::digest::digest(&ring::digest::SHA256, der.as_ref())
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The common name of the first certificate in `cert_pem` (a node's id).
pub fn subject_name(cert_pem: &str) -> Option<String> {
    let der = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .next()?
        .ok()?;
    let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref()).ok()?;
    let cn = cert.subject().iter_common_name().next()?;
    cn.as_str().ok().map(str::to_string)
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Whether `key_pem` is the private key of the first certificate in `cert_pem`.
pub fn pair_matches(cert_pem: &str, key_pem: &str) -> bool {
    let Ok(key) = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()) else {
        return false;
    };
    let Ok(chain) =
        CertificateDer::pem_slice_iter(cert_pem.as_bytes()).collect::<Result<Vec<_>, _>>()
    else {
        return false;
    };
    let Ok(signing) = provider().key_provider.load_private_key(key) else {
        return false;
    };
    rustls::sign::CertifiedKey::new(chain, signing)
        .keys_match()
        .is_ok()
}

/// The raw public key of a request's text (after the request's own signature was checked).
fn csr_public_key(csr_pem: &str) -> Option<Vec<u8>> {
    use x509_parser::prelude::FromDer;
    let (_, pem) = x509_parser::pem::parse_x509_pem(csr_pem.as_bytes()).ok()?;
    let (_, csr) =
        x509_parser::certification_request::X509CertificationRequest::from_der(&pem.contents)
            .ok()?;
    csr.verify_signature().ok()?;
    Some(csr.certification_request_info.subject_pki.raw.to_vec())
}

fn cert_public_key(cert_pem: &str) -> Option<Vec<u8>> {
    let der = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .next()?
        .ok()?;
    let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref()).ok()?;
    Some(cert.tbs_certificate.subject_pki.raw.to_vec())
}

/// Whether the request's public key is the one `key_pem` holds, told through a certificate-less check: the key signs nothing here, so
/// it is compared by what the request carries and by loading the key (a key that does not load is not a match).
fn csr_matches_key(csr_pem: &str, key_pem: &str) -> bool {
    let Ok(key) = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()) else {
        return false;
    };
    let Ok(signing) = provider().key_provider.load_private_key(key) else {
        return false;
    };
    let (Some(want), Some(have)) = (csr_public_key(csr_pem), signing.public_key()) else {
        return false;
    };
    // `public_key()` is the SubjectPublicKeyInfo DER, which is what the request's `subject_pki.raw` is.
    want == have.as_ref()
}

/// Why a certificate the hub sent was not accepted. A code for the log and the state file, never text for a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    /// Not a certificate, or it does not chain to the pinned CA, or it lasts longer than any the hub issues. Never right for the
    /// request: five in a row give it up.
    Chain,
    /// Time (ADR-0085 Amendment A): it has ended, or starts more than [`RENEW_FUTURE_START`] after this machine's clock. Keeps the
    /// request: once the clock is right, the same answer is accepted.
    Validity,
    /// It is not for client authentication only, or it is a CA.
    Usage,
    /// It names another node than this one.
    Node,
    /// It does not carry the public key of the request this machine sent.
    Key,
    /// It does not end later than the certificate in use (a renewal that gains nothing).
    NotNewer,
}

impl Rejected {
    /// Whether answers rejected for this reason count toward giving the request up ([`crate::link_renewal::MAX_REJECTED`] in a
    /// row): only those that can never be right for it. Time (`validity`) and `not_newer` keep it (ADR-0085 Amendment A).
    pub fn gives_up(self) -> bool {
        matches!(self, Self::Chain | Self::Usage | Self::Node | Self::Key)
    }

    /// Reads a code remembered in `renew-state`; `certificate`, from a version before the split, is `chain`.
    pub fn from_code(code: &str) -> Option<Self> {
        Some(match code {
            "chain" | "certificate" => Self::Chain,
            "validity" => Self::Validity,
            "usage" => Self::Usage,
            "node" => Self::Node,
            "key" => Self::Key,
            "not_newer" => Self::NotNewer,
            _ => return None,
        })
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Chain => "chain",
            Self::Validity => "validity",
            Self::Usage => "usage",
            Self::Node => "node",
            Self::Key => "key",
            Self::NotNewer => "not_newer",
        }
    }
}

/// Everything a renewed certificate must satisfy before it replaces the one in use: it chains to the pinned CA (`ca_pem`, every
/// certificate in it), has not ended and starts within [`RENEW_FUTURE_START`] of `now`, lasts no more than a year, is for client authentication only and not a CA, names exactly
/// `node_id`, carries the public key of the request this machine sent (and so of the persisted key), and ends later than `current_pem`.
pub fn check_renewed(
    ca_pem: &str,
    cert_pem: &str,
    node_id: &str,
    pending: &Pending,
    current_pem: &str,
    now: i64,
) -> Result<(), Rejected> {
    use crate::pairing_client::PairError;
    let mut roots = rustls::RootCertStore::empty();
    for c in CertificateDer::pem_slice_iter(ca_pem.as_bytes()) {
        let c = c.map_err(|_| Rejected::Chain)?;
        roots.add(c).map_err(|_| Rejected::Chain)?;
    }
    // Longer than any certificate the hub issues: never right, whatever the clock (pairing calls this `validity`).
    let (starts, ends) = validity(cert_pem).ok_or(Rejected::Chain)?;
    if ends.saturating_sub(starts) > crate::pairing_client::MAX_CERT_SECONDS {
        return Err(Rejected::Chain);
    }
    let csr_der = x509_parser::pem::parse_x509_pem(pending.csr_pem.as_bytes())
        .map_err(|_| Rejected::Key)?
        .1
        .contents;
    check_client_certificate(
        Arc::new(roots),
        cert_pem,
        node_id,
        &csr_der,
        now,
        RENEW_FUTURE_START,
    )
    .map_err(|e| match e {
        PairError::CertificateInvalid("usage") => Rejected::Usage,
        PairError::CertificateInvalid("subject") => Rejected::Node,
        PairError::CertificateInvalid("key") => Rejected::Key,
        PairError::CertificateInvalid("validity") => Rejected::Validity,
        _ => Rejected::Chain,
    })?;
    // The persisted private key is the one the certificate was made for (the request carries its public half, checked above).
    if !pair_matches(cert_pem, &pending.key_pem) || cert_public_key(cert_pem).is_none() {
        return Err(Rejected::Key);
    }
    let (_, current_ends) = validity(current_pem).ok_or(Rejected::Chain)?;
    if ends <= current_ends {
        return Err(Rejected::NotNewer);
    }
    Ok(())
}

/// What the hub said when it refused a renewal (`enroll.renew_refused`, parameter `reason`), grouped by what the node does about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The hub does not know the certificate, has revoked it, or it has ended: only a new join (with the owner) helps. Never retried.
    JoinAgain,
    /// A newer certificate of this machine exists: someone else may hold this machine's certificate. Never retried; logged as such.
    Superseded,
    /// A certificate was issued for this machine within the hour (or five answers were repeated within it). Tried again later.
    TooFrequent,
    /// Any other answer: treated as a temporary failure.
    Other(String),
}

impl Refusal {
    /// Reads the `reason` parameter of `enroll.renew_refused`.
    pub fn from_reason(reason: &str) -> Self {
        match reason {
            "unknown" | "revoked" | "expired" | "too_old" => Self::JoinAgain,
            "superseded" => Self::Superseded,
            "too_frequent" => Self::TooFrequent,
            other => Self::Other(other.to_string()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use rcgen::{
        BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
        IsCa, Issuer, KeyPair, KeyUsagePurpose,
    };
    use time::{Duration, OffsetDateTime};

    struct Ca {
        key: KeyPair,
        pem: String,
        params: CertificateParams,
    }

    fn ca(name: &str) -> Ca {
        let key = KeyPair::generate().unwrap();
        let mut p = CertificateParams::default();
        p.distinguished_name = dn(name);
        p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        p.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        let cert = p.self_signed(&key).unwrap();
        Ca {
            key,
            pem: cert.pem(),
            params: p,
        }
    }

    fn dn(name: &str) -> DistinguishedName {
        let mut d = DistinguishedName::new();
        d.push(DnType::CommonName, name);
        d
    }

    fn request() -> Pending {
        let key = KeyPair::generate().unwrap();
        let mut p = CertificateParams::default();
        p.distinguished_name = dn("device");
        let csr = p.serialize_request(&key).unwrap().pem().unwrap();
        Pending::new(&key.serialize_pem(), &csr)
    }

    /// A certificate for `request`'s key, signed by `ca`, with the knobs the checks look at.
    fn issue(
        ca: &Ca,
        request: &Pending,
        node: &str,
        life_secs: i64,
        usage: ExtendedKeyUsagePurpose,
        is_ca: bool,
    ) -> String {
        use rcgen::CertificateSigningRequestParams;
        let mut csr = CertificateSigningRequestParams::from_pem(&request.csr_pem).unwrap();
        let p = &mut csr.params;
        p.distinguished_name = dn(node);
        p.is_ca = if is_ca {
            IsCa::Ca(BasicConstraints::Unconstrained)
        } else {
            IsCa::ExplicitNoCa
        };
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        p.extended_key_usages = vec![usage];
        p.not_before = OffsetDateTime::now_utc() - Duration::minutes(5);
        p.not_after = OffsetDateTime::now_utc() + Duration::seconds(life_secs);
        let issuer = Issuer::new(ca.params.clone(), &ca.key);
        csr.signed_by(&issuer).unwrap().pem()
    }

    fn now() -> i64 {
        OffsetDateTime::now_utc().unix_timestamp()
    }

    const DAY: i64 = 86_400;

    fn good(ca: &Ca, node: &str) -> (Pending, String, String) {
        let current = request();
        let current_cert = issue(
            ca,
            &current,
            node,
            10 * DAY,
            ExtendedKeyUsagePurpose::ClientAuth,
            false,
        );
        let fresh = request();
        let cert = issue(
            ca,
            &fresh,
            node,
            90 * DAY,
            ExtendedKeyUsagePurpose::ClientAuth,
            false,
        );
        (fresh, cert, current_cert)
    }

    #[test]
    fn a_certificate_is_due_when_a_third_of_its_own_life_is_left() {
        let ca = ca("ca");
        let r = request();
        let cert = issue(
            &ca,
            &r,
            "m",
            90 * DAY,
            ExtendedKeyUsagePurpose::ClientAuth,
            false,
        );
        let (from, to) = validity(&cert).unwrap();
        let life = to - from;
        // Exactly a third left is due; one second more is not.
        assert_eq!(due(&cert, to - life / 3), Some(true));
        assert_eq!(due(&cert, to - life / 3 - 1), Some(false));
        assert_eq!(due(&cert, from), Some(false));
        // Ended is still due (the caller learns the hub refuses it).
        assert_eq!(due(&cert, to + 5), Some(true));
        // The fraction is of THIS certificate's life, not of a constant 90 days.
        let short = issue(&ca, &r, "m", 3, ExtendedKeyUsagePurpose::ClientAuth, false);
        let (f, t) = validity(&short).unwrap();
        assert_eq!(due(&short, t - (t - f) / 3), Some(true));
        assert_eq!(due(&short, f + 1), Some(false));
        assert_eq!(due("not a certificate", 0), None);
    }

    #[test]
    fn pending_is_one_text_that_decodes_only_when_the_request_is_the_keys_own() {
        let p = request();
        assert_eq!(Pending::decode(&p.encode()), Some(p.clone()));
        // The request of another key does not match this key.
        let other = request();
        let mixed = Pending {
            key_pem: p.key_pem.clone(),
            csr_pem: other.csr_pem,
            for_cert: String::new(),
        };
        assert_eq!(Pending::decode(&mixed.encode()), None);
        assert_eq!(Pending::decode(&p.key_pem), None);
        assert_eq!(Pending::decode(""), None);
        assert_eq!(Pending::decode(&p.csr_pem), None);
        // A key is never printed.
        assert!(!format!("{p:?}").contains("PRIVATE"));
    }

    #[test]
    fn a_good_answer_passes() {
        let ca = ca("ca");
        let (fresh, cert, current) = good(&ca, "m");
        assert_eq!(
            check_renewed(&ca.pem, &cert, "m", &fresh, &current, now()),
            Ok(())
        );
        assert!(pair_matches(&cert, &fresh.key_pem));
    }

    #[test]
    fn an_answer_is_rejected_when_the_persisted_key_is_not_the_requests_key() {
        // A request and key that do not belong together cannot be built by `decode`, but the checks must not rely on that: the
        // certificate is for the request's key, and the key kept is another.
        let ca = ca("ca");
        let (fresh, cert, current) = good(&ca, "m");
        let other = request();
        let inconsistent = Pending {
            key_pem: other.key_pem,
            csr_pem: fresh.csr_pem,
            for_cert: String::new(),
        };
        assert_eq!(
            check_renewed(&ca.pem, &cert, "m", &inconsistent, &current, now()),
            Err(Rejected::Key)
        );
    }

    #[test]
    fn a_pending_request_is_bound_to_the_certificate_it_was_made_for() {
        let ca = ca("ca");
        let (_, cert, current) = good(&ca, "m");
        let p = request().bound(&current);
        assert_eq!(Pending::decode(&p.encode()), Some(p.clone()));
        assert_eq!(p.for_cert, fingerprint(&current));
        assert_ne!(p.for_cert, fingerprint(&cert));
        assert!(!p.for_cert.is_empty());
        assert_eq!(subject_name(&cert).as_deref(), Some("m"));
    }

    #[test]
    fn an_answer_for_another_node_is_rejected() {
        let ca = ca("ca");
        let (fresh, cert, current) = good(&ca, "other");
        assert_eq!(
            check_renewed(&ca.pem, &cert, "m", &fresh, &current, now()),
            Err(Rejected::Node)
        );
    }

    #[test]
    fn an_answer_for_another_key_is_rejected() {
        let ca = ca("ca");
        let (fresh, _, current) = good(&ca, "m");
        // A certificate for a key this machine never made.
        let stranger = request();
        let cert = issue(
            &ca,
            &stranger,
            "m",
            90 * DAY,
            ExtendedKeyUsagePurpose::ClientAuth,
            false,
        );
        assert_eq!(
            check_renewed(&ca.pem, &cert, "m", &fresh, &current, now()),
            Err(Rejected::Key)
        );
        // The persisted key is checked too, not only the request: a request kept with another key is not a pending renewal at all.
        let swapped = Pending {
            key_pem: stranger.key_pem,
            csr_pem: fresh.csr_pem.clone(),
            for_cert: String::new(),
        };
        assert_eq!(Pending::decode(&swapped.encode()), None);
    }

    #[test]
    fn an_answer_not_chained_to_the_pinned_ca_is_rejected() {
        let ca1 = ca("ca");
        let ca2 = ca("ca");
        let (fresh, cert, current) = good(&ca2, "m");
        assert_eq!(
            check_renewed(&ca1.pem, &cert, "m", &fresh, &current, now()),
            Err(Rejected::Chain)
        );
    }

    #[test]
    fn an_answer_with_the_wrong_usage_or_a_ca_is_rejected() {
        let ca = ca("ca");
        let (fresh, _, current) = good(&ca, "m");
        let server = issue(
            &ca,
            &fresh,
            "m",
            90 * DAY,
            ExtendedKeyUsagePurpose::ServerAuth,
            false,
        );
        assert_eq!(
            check_renewed(&ca.pem, &server, "m", &fresh, &current, now()),
            Err(Rejected::Chain)
        );
        let as_ca = issue(
            &ca,
            &fresh,
            "m",
            90 * DAY,
            ExtendedKeyUsagePurpose::ClientAuth,
            true,
        );
        assert!(check_renewed(&ca.pem, &as_ca, "m", &fresh, &current, now()).is_err());
    }

    #[test]
    fn an_answer_that_is_ended_absurdly_long_or_not_newer_is_rejected() {
        let ca = ca("ca");
        let (fresh, _, current) = good(&ca, "m");
        let ended = issue(
            &ca,
            &fresh,
            "m",
            -3600,
            ExtendedKeyUsagePurpose::ClientAuth,
            false,
        );
        assert_eq!(
            check_renewed(&ca.pem, &ended, "m", &fresh, &current, now()),
            Err(Rejected::Validity)
        );
        let decades = issue(
            &ca,
            &fresh,
            "m",
            40 * 366 * DAY,
            ExtendedKeyUsagePurpose::ClientAuth,
            false,
        );
        assert_eq!(
            check_renewed(&ca.pem, &decades, "m", &fresh, &current, now()),
            Err(Rejected::Chain)
        );
        // Ends no later than the certificate in use: pointless, kept out.
        let short = issue(
            &ca,
            &fresh,
            "m",
            2 * DAY,
            ExtendedKeyUsagePurpose::ClientAuth,
            false,
        );
        assert_eq!(
            check_renewed(&ca.pem, &short, "m", &fresh, &current, now()),
            Err(Rejected::NotNewer)
        );
        assert_eq!(
            check_renewed(&ca.pem, "garbage", "m", &fresh, &current, now()),
            Err(Rejected::Chain)
        );
    }

    /// ADR-0085 Amendment A (F1340): a machine whose clock is behind the hub's takes a renewed certificate that starts up to a day in
    /// its future, and calls one further ahead (or ended) `validity`, which keeps the request; pairing still wants "valid now".
    #[test]
    fn an_answer_that_starts_up_to_a_day_after_this_machines_clock_is_accepted() {
        let other = ca("other");
        let ca = ca("ca");
        let (fresh, cert, current) = good(&ca, "m");
        // The hub starts it 5 minutes before its own now.
        for behind in [0, 15 * 60, 6 * 3600, 23 * 3600] {
            assert_eq!(
                check_renewed(&ca.pem, &cert, "m", &fresh, &current, now() - behind),
                Ok(()),
                "{behind} s behind"
            );
        }
        for behind in [RENEW_FUTURE_START + 3600, 30 * 3600] {
            assert_eq!(
                check_renewed(&ca.pem, &cert, "m", &fresh, &current, now() - behind),
                Err(Rejected::Validity),
                "{behind} s behind"
            );
        }
        // Another CA is still `chain` with the clock behind: the start is not what makes it fail.
        let (o_fresh, o_cert, o_current) = good(&other, "m");
        assert_eq!(
            check_renewed(
                &ca.pem,
                &o_cert,
                "m",
                &o_fresh,
                &o_current,
                now() - 6 * 3600
            ),
            Err(Rejected::Chain)
        );
    }

    #[test]
    fn only_answers_that_can_never_be_right_give_the_request_up() {
        for r in [
            Rejected::Chain,
            Rejected::Usage,
            Rejected::Node,
            Rejected::Key,
        ] {
            assert!(r.gives_up(), "{r:?}");
            assert_eq!(Rejected::from_code(r.code()), Some(r));
        }
        for r in [Rejected::Validity, Rejected::NotNewer] {
            assert!(!r.gives_up(), "{r:?}");
            assert_eq!(Rejected::from_code(r.code()), Some(r));
        }
        // Remembered by a version before the split.
        assert_eq!(Rejected::from_code("certificate"), Some(Rejected::Chain));
        assert_eq!(Rejected::from_code("x"), None);
    }

    #[test]
    fn refusal_reasons_group_by_what_the_node_does() {
        for r in ["unknown", "revoked", "expired", "too_old"] {
            assert_eq!(Refusal::from_reason(r), Refusal::JoinAgain, "{r}");
        }
        assert_eq!(Refusal::from_reason("superseded"), Refusal::Superseded);
        assert_eq!(Refusal::from_reason("too_frequent"), Refusal::TooFrequent);
        assert_eq!(
            Refusal::from_reason("something new"),
            Refusal::Other("something new".into())
        );
    }
}
