//! The hub's certificate authority for device enrollment (ADR-0015). A node generates its own key pair and sends
//! a certificate signing request, so its private key never leaves it. The hub decides the identity: whatever
//! name the request asks for is ignored, and the certificate carries the node id the hub approved.

use std::fmt;

use rcgen::{
    BasicConstraints, CertificateParams, CertificateSigningRequestParams, DistinguishedName,
    DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose, SerialNumber,
};
use time::{Duration, OffsetDateTime};

#[derive(Debug)]
pub struct CaError(String);

impl fmt::Display for CaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ca: {}", self.0)
    }
}

impl std::error::Error for CaError {}

impl From<rcgen::Error> for CaError {
    fn from(e: rcgen::Error) -> Self {
        Self(e.to_string())
    }
}

/// Validity of an issued device certificate. Short-lived on purpose. A machine renews it over its own connection and the hub replaces its
/// own server certificate before it ends (docs/adr/0085-machine-and-hub-certificates-renew-themselves.md); the hub side is built, the
/// machine side is not yet, so until then a machine must still be enrolled again before this many days pass.
pub const DEVICE_CERT_DAYS: i64 = 90;
/// `agora doctor` and `agora hub machines` warn when a certificate has fewer days than this left.
pub const EXPIRY_WARN_DAYS: i64 = 30;
const CA_DAYS: i64 = 3650;
/// Producer id of events the hub records itself.
pub const RESERVED_HUB_ID: &str = "hub";

pub struct Ca {
    cert_pem: String,
    key: KeyPair,
    /// How long node and server certificates last. [`DEVICE_CERT_DAYS`] unless a test shortens it.
    lifetime: Duration,
}

/// What the hub hands back to an enrolled device.
pub struct IssuedCert {
    pub cert_pem: String,
    pub serial_hex: String,
}

/// The common name of a hub's own CA (`agora hub` makes it once). A node refuses a preview of a port that shows a certificate issued
/// under it (ADR-0088), so a hub on the same machine is never reached as a loopback peer.
pub const HUB_CA_NAME: &str = "Agora hub CA";

fn name(cn: &str) -> DistinguishedName {
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, cn);
    dn
}

impl Ca {
    pub fn generate(common_name: &str) -> Result<Self, CaError> {
        let key = KeyPair::generate()?;
        let mut p = CertificateParams::default();
        p.distinguished_name = name(common_name);
        p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        p.not_before = OffsetDateTime::now_utc() - Duration::minutes(5);
        p.not_after = OffsetDateTime::now_utc() + Duration::days(CA_DAYS);
        let cert = p.self_signed(&key)?;
        Ok(Self {
            cert_pem: cert.pem(),
            key,
            lifetime: Duration::days(DEVICE_CERT_DAYS),
        })
    }

    pub fn from_pem(cert_pem: &str, key_pem: &str) -> Result<Self, CaError> {
        let key = KeyPair::from_pem(key_pem)?;
        // Parsing proves the pair is usable for signing before the hub depends on it.
        Issuer::from_ca_cert_pem(cert_pem, &key)?;
        // ...and that the key is the one the certificate was issued for.
        let der = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
            .map_err(|e| CaError(e.to_string()))?
            .1;
        let cert = der.parse_x509().map_err(|e| CaError(e.to_string()))?;
        if cert.public_key().subject_public_key.data.as_ref() != key.public_key_raw() {
            return Err(CaError("the key does not belong to the certificate".into()));
        }
        Ok(Self {
            cert_pem: cert_pem.to_string(),
            key,
            lifetime: Duration::days(DEVICE_CERT_DAYS),
        })
    }

    /// For tests only: the certificates this CA issues from now on last `lifetime` instead of [`DEVICE_CERT_DAYS`]. Nothing in
    /// the product calls it, so production behaviour is unchanged.
    #[must_use]
    pub fn with_lifetime_for_tests(mut self, lifetime: std::time::Duration) -> Self {
        self.lifetime = Duration::try_from(lifetime).unwrap_or(Duration::days(DEVICE_CERT_DAYS));
        self
    }

    /// How long the certificates this CA issues are valid, in milliseconds.
    pub fn lifetime_ms(&self) -> u64 {
        u64::try_from(self.lifetime.whole_milliseconds()).unwrap_or(0)
    }

    pub fn cert_pem(&self) -> &str {
        &self.cert_pem
    }

    /// The private key, for the hub to store (it never leaves the hub).
    pub fn key_pem(&self) -> String {
        self.key.serialize_pem()
    }

    /// Signs a node's CSR as `node_id`, valid for client authentication only.
    pub fn issue_node(&self, node_id: &str, csr_pem: &str) -> Result<IssuedCert, CaError> {
        // "hub" is the producer id of facts the hub itself records (a node going offline); no node may use it.
        if !valid_node_id(node_id) {
            return Err(CaError("node id must be 1-64 of [A-Za-z0-9._-]".into()));
        }
        let mut csr = CertificateSigningRequestParams::from_pem(csr_pem)?;
        let p = &mut csr.params;
        // The device does not get to choose who it is.
        p.distinguished_name = name(node_id);
        p.subject_alt_names = Vec::new();
        p.is_ca = IsCa::ExplicitNoCa;
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        p.use_authority_key_identifier_extension = true;
        // A fresh random serial for every certificate (RFC 5280 4.1.2.2): left unset, rcgen derives it from the public key, so two
        // certificates for one key would share a serial and the second could not be recorded (`devices.serial` is the key).
        p.serial_number = Some(random_serial()?);
        p.not_before = OffsetDateTime::now_utc() - Duration::minutes(5);
        p.not_after = OffsetDateTime::now_utc() + self.lifetime;
        let issuer = Issuer::from_ca_cert_pem(&self.cert_pem, &self.key)?;
        let cert = csr.signed_by(&issuer)?;
        let serial_hex = x509_parser::parse_x509_certificate(cert.der())
            .map(|(_, c)| c.raw_serial_as_string().replace(':', ""))
            .map_err(|e| CaError(e.to_string()))?;
        Ok(IssuedCert {
            serial_hex,
            cert_pem: cert.pem(),
        })
    }
}

impl Ca {
    /// A server certificate for the hub's own TLS endpoint, signed by this CA. Returns `(cert_pem, key_pem)`.
    /// Nodes pin this CA; browsers need a publicly trusted certificate instead (a deployment choice).
    pub fn issue_server(&self, dns_names: &[String]) -> Result<(String, String), CaError> {
        if dns_names.is_empty() {
            return Err(CaError(
                "a server certificate needs at least one name".into(),
            ));
        }
        let key = KeyPair::generate()?;
        let mut p = CertificateParams::new(dns_names.to_vec())?;
        p.distinguished_name = name(&dns_names[0]);
        p.is_ca = IsCa::ExplicitNoCa;
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        p.use_authority_key_identifier_extension = true;
        p.not_before = OffsetDateTime::now_utc() - Duration::minutes(5);
        p.not_after = OffsetDateTime::now_utc() + self.lifetime;
        let issuer = Issuer::from_ca_cert_pem(&self.cert_pem, &self.key)?;
        let cert = p.signed_by(&key, &issuer)?;
        Ok((cert.pem(), key.serialize_pem()))
    }
}

/// 126 random bits as a positive 16-byte serial: the top bit clear (positive), the next one set (no leading zero byte to drop).
fn random_serial() -> Result<SerialNumber, CaError> {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).map_err(|e| CaError(e.to_string()))?;
    b[0] = (b[0] & 0x7f) | 0x40;
    Ok(SerialNumber::from_slice(&b))
}

/// When a PEM certificate stops being valid, in seconds since 1970 (the first certificate in the text).
pub fn cert_not_after_secs(pem: &str) -> Result<i64, CaError> {
    let der = x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .map_err(|e| CaError(e.to_string()))?
        .1;
    let cert = der.parse_x509().map_err(|e| CaError(e.to_string()))?;
    Ok(cert.validity().not_after.timestamp())
}

/// The names a PEM certificate carries (its alternative names: DNS names as written, addresses in their shortest text), in the order
/// they are in the certificate. For `agora doctor`: which names the hub's certificate has.
pub fn cert_names(pem: &str) -> Result<Vec<String>, CaError> {
    use x509_parser::extensions::GeneralName;
    let der = x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .map_err(|e| CaError(e.to_string()))?
        .1;
    let cert = der.parse_x509().map_err(|e| CaError(e.to_string()))?;
    let Ok(Some(san)) = cert.subject_alternative_name() else {
        return Ok(Vec::new());
    };
    Ok(san
        .value
        .general_names
        .iter()
        .filter_map(|n| match n {
            GeneralName::DNSName(d) => Some((*d).to_string()),
            GeneralName::IPAddress(b) => match b.len() {
                4 => <[u8; 4]>::try_from(*b)
                    .ok()
                    .map(|a| std::net::IpAddr::from(a).to_string()),
                16 => <[u8; 16]>::try_from(*b)
                    .ok()
                    .map(|a| std::net::IpAddr::from(a).to_string()),
                _ => None,
            },
            _ => None,
        })
        .collect())
}

/// Where a certificate stands against [`EXPIRY_WARN_DAYS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expiry {
    /// Already past its end, by this many whole days (0 means today).
    Expired(i64),
    /// Valid, with fewer than [`EXPIRY_WARN_DAYS`] whole days left.
    Soon(i64),
    /// Valid, with this many whole days left.
    Fine(i64),
}

pub fn expiry(not_after_secs: i64, now_secs: i64) -> Expiry {
    let left = not_after_secs - now_secs;
    if left <= 0 {
        Expiry::Expired((-left) / 86_400)
    } else if left < EXPIRY_WARN_DAYS * 86_400 {
        Expiry::Soon(left / 86_400)
    } else {
        Expiry::Fine(left / 86_400)
    }
}

/// The rules for a node id: 1-64 of `[A-Za-z0-9._-]`, and not `hub` (the producer id of facts the hub itself records).
pub fn valid_node_id(node_id: &str) -> bool {
    node_id != RESERVED_HUB_ID
        && !node_id.is_empty()
        && node_id.len() <= 64
        && node_id.chars().all(valid_id_char)
}

/// The DER of a CSR given as PEM, after checking its signature (the requester holds the key it asks to be certified). Pairing
/// hashes this, so the hash is of the bytes the machine signed, not of its text.
pub fn csr_der(csr_pem: &str) -> Result<Vec<u8>, CaError> {
    CertificateSigningRequestParams::from_pem(csr_pem)?;
    let (_, pem) =
        x509_parser::pem::parse_x509_pem(csr_pem.as_bytes()).map_err(|e| CaError(e.to_string()))?;
    Ok(pem.contents)
}

fn valid_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// Node side: a fresh key pair and the CSR to send to the hub. Returns `(private_key_pem, csr_pem)`.
/// SHA-256 of the public key a certificate signing request carries, after checking that the request is signed by that key (proof of
/// possession). ADR-0085 keeps it with each certificate a renewal issued, to recognize the same request asked again.
pub fn csr_key_hash(csr_pem: &str) -> Result<[u8; 32], CaError> {
    use sha2::{Digest, Sha256};
    use x509_parser::prelude::FromDer;
    let der = x509_parser::pem::parse_x509_pem(csr_pem.as_bytes())
        .map_err(|e| CaError(e.to_string()))?
        .1;
    let (_, csr) =
        x509_parser::certification_request::X509CertificationRequest::from_der(&der.contents)
            .map_err(|e| CaError(e.to_string()))?;
    csr.verify_signature()
        .map_err(|_| CaError("the request is not signed by its own key".into()))?;
    Ok(Sha256::digest(
        csr.certification_request_info
            .subject_pki
            .subject_public_key
            .data
            .as_ref(),
    )
    .into())
}

pub fn new_device_request() -> Result<(String, String), CaError> {
    let key = KeyPair::generate()?;
    let mut p = CertificateParams::default();
    p.distinguished_name = name("device");
    let csr = p.serialize_request(&key)?;
    Ok((key.serialize_pem(), csr.pem()?))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use x509_parser::prelude::{FromDer, X509Certificate};

    fn parse(pem: &str) -> Vec<u8> {
        ::x509_parser::pem::parse_x509_pem(pem.as_bytes())
            .unwrap()
            .1
            .contents
    }

    /// The identity of an issued node certificate is the hub's: a CSR may ask for any subject, any alternative names, CA powers or a
    /// server's key usage, and none of it reaches the certificate (pairing and enrollment both issue through `issue_node`).
    #[test]
    fn the_certificate_carries_the_hub_chosen_identity_whatever_the_csr_asks_for() {
        use rcgen::{
            BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, SanType,
        };
        let ca = Ca::generate("hub ca").unwrap();
        let key = KeyPair::generate().unwrap();
        let mut p =
            CertificateParams::new(vec!["evil.example".to_string(), "hub".to_string()]).unwrap();
        p.subject_alt_names
            .push(SanType::IpAddress("10.0.0.1".parse().unwrap()));
        p.distinguished_name = DistinguishedName::new();
        p.distinguished_name.push(DnType::CommonName, "hub");
        p.distinguished_name
            .push(DnType::OrganizationName, "evil corp");
        p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let csr = p.serialize_request(&key).unwrap().pem().unwrap();
        // The CSR really carries what the attacker chose (so the test would notice if it stopped doing so).
        let asked = CertificateSigningRequestParams::from_pem(&csr).unwrap();
        assert_eq!(asked.params.subject_alt_names.len(), 3);

        let issued = ca.issue_node("laptop", &csr).unwrap();
        let der = parse(&issued.cert_pem);
        let (_, cert) = X509Certificate::from_der(&der).unwrap();
        // Subject: exactly the node id the hub chose, nothing else.
        let cns: Vec<_> = cert
            .subject()
            .iter_common_name()
            .map(|c| c.as_str().unwrap().to_string())
            .collect();
        assert_eq!(cns, ["laptop"]);
        assert_eq!(cert.subject().iter_attributes().count(), 1);
        // Alternative names: none at all (the hub sets the empty list).
        assert!(
            cert.subject_alternative_name().unwrap().is_none(),
            "an alternative name from the CSR reached the certificate"
        );
        // Powers: client authentication only, and not a CA.
        let eku = cert.extended_key_usage().unwrap().unwrap().value;
        assert!(eku.client_auth && !eku.server_auth && !eku.any && !eku.code_signing);
        assert!(!cert.is_ca());
    }

    /// F850: a serial is unique per certificate, not derived from the key: one request signed twice gets two serials, and the
    /// hub can record a re-pairing with the same key (`devices.serial` is the table's key).
    #[test]
    fn every_certificate_gets_its_own_serial_even_for_one_key() {
        let ca = Ca::generate("ca").unwrap();
        let (_, csr) = new_device_request().unwrap();
        let a = ca.issue_node("vps", &csr).unwrap().serial_hex;
        let b = ca.issue_node("vps", &csr).unwrap().serial_hex;
        assert_ne!(a, b);
        let store = crate::store::SqliteStore::open_in_memory().unwrap();
        let mut e = crate::enroll::Enrollment::new(&store);
        let code = e.create_code(0).unwrap();
        let old = e.redeem(&ca, &code, "vps", &csr, 1).unwrap();
        let new = e.issue_paired(&ca, "vps", &csr, true, 2).unwrap();
        assert!(!e.is_active(&old.serial_hex).unwrap());
        assert!(e.is_active(&new.serial_hex).unwrap());
    }

    #[test]
    fn expiry_says_expired_soon_or_fine_by_whole_days() {
        let day = 86_400;
        assert_eq!(expiry(1000, 1000), Expiry::Expired(0));
        assert_eq!(expiry(1000, 1000 + 3 * day), Expiry::Expired(3));
        assert_eq!(expiry(10 * day, 0), Expiry::Soon(10));
        assert_eq!(expiry(30 * day, 0), Expiry::Fine(30));
        assert_eq!(expiry(30 * day - 1, 0), Expiry::Soon(29));
    }

    #[test]
    fn the_certificate_end_is_readable_and_the_test_lifetime_shortens_it() {
        let (_, csr) = new_device_request().unwrap();
        let ca = Ca::generate("ca").unwrap();
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let normal = cert_not_after_secs(&ca.issue_node("n", &csr).unwrap().cert_pem).unwrap();
        assert!((normal - now - DEVICE_CERT_DAYS * 86_400).abs() < 10);
        let ca = ca.with_lifetime_for_tests(std::time::Duration::from_secs(60));
        let short = cert_not_after_secs(&ca.issue_node("n", &csr).unwrap().cert_pem).unwrap();
        assert!((short - now - 60).abs() < 10);
        assert_eq!(ca.lifetime_ms(), 60_000);
    }

    #[test]
    fn issues_a_client_certificate_the_ca_verifies() {
        let ca = Ca::generate("Agora hub CA").unwrap();
        let (_, csr) = new_device_request().unwrap();
        let issued = ca.issue_node("laptop-1", &csr).unwrap();

        let ca_der = parse(ca.cert_pem());
        let (_, ca_cert) = X509Certificate::from_der(&ca_der).unwrap();
        let der = parse(&issued.cert_pem);
        let (_, cert) = X509Certificate::from_der(&der).unwrap();

        cert.verify_signature(Some(ca_cert.public_key())).unwrap();
        assert_eq!(cert.subject().to_string(), "CN=laptop-1");
        assert!(!cert.is_ca());
        let eku = cert.extended_key_usage().unwrap().unwrap().value;
        assert!(eku.client_auth && !eku.server_auth);
        assert!(cert.validity().is_valid());
        assert!(!issued.serial_hex.is_empty());
    }

    #[test]
    fn the_device_cannot_choose_its_identity_or_become_a_ca() {
        let ca = Ca::generate("ca").unwrap();
        // A hostile CSR asking to be a CA called "admin".
        let key = KeyPair::generate().unwrap();
        let mut p = CertificateParams::default();
        p.distinguished_name = name("admin");
        p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let csr = p.serialize_request(&key).unwrap().pem().unwrap();
        let issued = ca.issue_node("n1", &csr).unwrap();
        let der = parse(&issued.cert_pem);
        let (_, cert) = X509Certificate::from_der(&der).unwrap();
        assert_eq!(cert.subject().to_string(), "CN=n1");
        assert!(!cert.is_ca());
    }

    #[test]
    fn rejects_bad_node_ids_and_garbage_requests() {
        let ca = Ca::generate("ca").unwrap();
        let (_, csr) = new_device_request().unwrap();
        for bad in ["", "has space", "a/b", "hub", &"x".repeat(65)] {
            assert!(ca.issue_node(bad, &csr).is_err(), "{bad:?}");
        }
        assert!(ca.issue_node("ok", "not a csr").is_err());
    }

    #[test]
    fn a_ca_reloads_from_its_stored_pem_and_still_signs() {
        let ca = Ca::generate("ca").unwrap();
        let again = Ca::from_pem(ca.cert_pem(), &ca.key_pem()).unwrap();
        let (_, csr) = new_device_request().unwrap();
        let issued = again.issue_node("n", &csr).unwrap();
        let ca_der = parse(ca.cert_pem());
        let (_, ca_cert) = X509Certificate::from_der(&ca_der).unwrap();
        let der = parse(&issued.cert_pem);
        let (_, cert) = X509Certificate::from_der(&der).unwrap();
        cert.verify_signature(Some(ca_cert.public_key())).unwrap();
        // A key that does not belong to the certificate is refused.
        let other = Ca::generate("other").unwrap();
        assert!(Ca::from_pem(ca.cert_pem(), &other.key_pem()).is_err());
    }
}
