//! Signed leaves identify the existing issuer, including after loading the same CA from PEM.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use agora_hub::ca::{new_device_request, Ca};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU32, Ordering},
};
use x509_parser::{extensions::ParsedExtension, prelude::*};

fn der(pem: &str) -> Vec<u8> {
    x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .unwrap()
        .1
        .contents
}

fn check_issuer(ca_pem: &str, leaf_pem: &str) {
    let ca_der = der(ca_pem);
    let (_, ca) = X509Certificate::from_der(&ca_der).unwrap();
    let subject_id = ca
        .extensions()
        .iter()
        .find_map(|extension| match extension.parsed_extension() {
            ParsedExtension::SubjectKeyIdentifier(id) => Some(id.0),
            _ => None,
        })
        .expect("CA has a subject key identifier");
    let leaf_der = der(leaf_pem);
    let (_, leaf) = X509Certificate::from_der(&leaf_der).unwrap();
    let ids: Vec<_> = leaf
        .extensions()
        .iter()
        .filter_map(|extension| match extension.parsed_extension() {
            ParsedExtension::AuthorityKeyIdentifier(id) => Some((
                extension.critical,
                id.key_identifier.as_ref().map(|id| id.0),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        ids,
        vec![(false, Some(subject_id))],
        "one non-critical issuer identifier must match the actual CA"
    );
    leaf.verify_signature(Some(ca.public_key())).unwrap();
}

#[test]
fn server_certificates_identify_the_issuer_without_replacing_the_ca() {
    let original = Ca::generate("fixture CA").unwrap();
    let ca_pem = original.cert_pem().to_owned();
    let loaded = Ca::from_pem(&ca_pem, &original.key_pem()).unwrap();
    for ca in [&original, &loaded] {
        let (server, _) = ca
            .issue_server(&["localhost".into(), "127.0.0.1".into()])
            .unwrap();
        check_issuer(&ca_pem, &server);
        assert_eq!(ca.cert_pem(), ca_pem);
    }
}

#[test]
fn node_certificates_identify_the_issuer_without_replacing_the_ca() {
    let original = Ca::generate("fixture CA").unwrap();
    let ca_pem = original.cert_pem().to_owned();
    let loaded = Ca::from_pem(&ca_pem, &original.key_pem()).unwrap();
    for ca in [&original, &loaded] {
        let (_, csr) = new_device_request().unwrap();
        let node = ca.issue_node("fixture-node", &csr).unwrap();
        check_issuer(&ca_pem, &node.cert_pem);
        assert_eq!(ca.cert_pem(), ca_pem);
    }
}

#[test]
fn a_loaded_ca_keeps_its_preexisting_custom_subject_key_identifier() {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyIdMethod, KeyPair, KeyUsagePurpose};
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::default();
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    params.key_identifier_method = KeyIdMethod::PreSpecified(b"existing-custom-key-id".to_vec());
    let root = params.self_signed(&key).unwrap();
    let ca = Ca::from_pem(&root.pem(), &key.serialize_pem()).unwrap();
    let (server, _) = ca.issue_server(&["localhost".into()]).unwrap();
    let (_, csr) = new_device_request().unwrap();
    let node = ca.issue_node("fixture-node", &csr).unwrap();
    check_issuer(ca.cert_pem(), &server);
    check_issuer(ca.cert_pem(), &node.cert_pem);
    assert_eq!(ca.cert_pem(), root.pem());
}

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn independent_strict_verification_when_openssl_is_available() {
    let version = match Command::new("openssl").arg("version").output() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipped external verification: OpenSSL unavailable; portable issuer checks still run");
            return;
        }
        other => other.unwrap(),
    };
    assert!(version.status.success());
    if !version.stdout.starts_with(b"OpenSSL ") {
        eprintln!("skipped external verification: openssl command is another implementation");
        return;
    }
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "agora-ca-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    fs::create_dir_all(&fixture.0).unwrap();
    let ca = Ca::generate("fixture CA").unwrap();
    let loaded = Ca::from_pem(ca.cert_pem(), &ca.key_pem()).unwrap();
    let (server, _) = loaded.issue_server(&["localhost".into()]).unwrap();
    let (_, csr) = new_device_request().unwrap();
    let client = loaded.issue_node("fixture-node", &csr).unwrap();
    fs::write(fixture.0.join("ca.pem"), ca.cert_pem()).unwrap();
    fs::write(fixture.0.join("server.pem"), server).unwrap();
    fs::write(fixture.0.join("client.pem"), client.cert_pem).unwrap();
    let verify = |leaf: &str, purpose: &str, hostname: Option<&str>| {
        let mut command = Command::new("openssl");
        command
            .args(["verify", "-x509_strict", "-purpose", purpose, "-CAfile"])
            .arg(fixture.0.join("ca.pem"));
        if let Some(hostname) = hostname {
            command.args(["-verify_hostname", hostname]);
        }
        command.arg(fixture.0.join(leaf)).output().unwrap()
    };
    for (leaf, purpose, host) in [
        ("server.pem", "sslserver", Some("localhost")),
        ("client.pem", "sslclient", None),
    ] {
        let result = verify(leaf, purpose, host);
        assert!(result.status.success(), "{leaf}: {result:?}");
    }
    assert!(!verify("server.pem", "sslserver", Some("another.invalid"))
        .status
        .success());
    assert!(!verify("client.pem", "sslserver", None).status.success());
}
