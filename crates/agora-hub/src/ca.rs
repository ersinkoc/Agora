//! The hub's certificate authority for device enrollment (ADR-0015). A node generates its own key pair and sends
//! a certificate signing request, so its private key never leaves it. The hub decides the identity: whatever
//! name the request asks for is ignored, and the certificate carries the node id the hub approved.

use std::fmt;

use rcgen::{
    BasicConstraints, CertificateParams, CertificateSigningRequestParams, DistinguishedName,
    DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose,
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

/// Validity of an issued device certificate. Short-lived on purpose; renewal is automatic while enrolled.
pub const DEVICE_CERT_DAYS: i64 = 90;
const CA_DAYS: i64 = 3650;
/// Producer id of events the hub records itself.
pub const RESERVED_HUB_ID: &str = "hub";

pub struct Ca {
    cert_pem: String,
    key: KeyPair,
}

/// What the hub hands back to an enrolled device.
pub struct IssuedCert {
    pub cert_pem: String,
    pub serial_hex: String,
}

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
        })
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
        if node_id == RESERVED_HUB_ID
            || node_id.is_empty()
            || node_id.len() > 64
            || !node_id.chars().all(valid_id_char)
        {
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
        p.not_before = OffsetDateTime::now_utc() - Duration::minutes(5);
        p.not_after = OffsetDateTime::now_utc() + Duration::days(DEVICE_CERT_DAYS);
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
        p.not_before = OffsetDateTime::now_utc() - Duration::minutes(5);
        p.not_after = OffsetDateTime::now_utc() + Duration::days(DEVICE_CERT_DAYS);
        let issuer = Issuer::from_ca_cert_pem(&self.cert_pem, &self.key)?;
        let cert = p.signed_by(&key, &issuer)?;
        Ok((cert.pem(), key.serialize_pem()))
    }
}

fn valid_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// Node side: a fresh key pair and the CSR to send to the hub. Returns `(private_key_pem, csr_pem)`.
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
