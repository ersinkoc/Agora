//! Adapter registry verification (ADR-0018). The registry is a static index plus packages; nothing it serves is
//! trusted until it verifies here, so a compromised mirror cannot inject code.
//!
//! - The **index** is signed by the registry's root key (the node pins it) and carries a version and an expiry. A
//!   node rejects an expired index and one older than the last it saw (rollback protection). The index lists
//!   publishers (Ed25519 public keys, some revoked) and packages (publisher, hash).
//! - A **package** is accepted only if its SHA-256 matches the index entry and its detached signature verifies under
//!   the listed publisher's key, and that publisher is not revoked.
//!
//! Signed data is the exact bytes of the `payload` string in an envelope, so there is no canonicalization to get
//! wrong. Installing a third-party package additionally needs the user's explicit approval (ADR-0018); that is the
//! caller's step, after this passes.

use std::collections::BTreeMap;

use ring::signature::{UnparsedPublicKey, ED25519};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// Not the envelope or JSON this code expects.
    Malformed,
    /// The signature does not verify.
    BadSignature,
    /// Past its `expires_ms`.
    Expired,
    /// Older than the newest index this node has seen.
    Rollback {
        seen: u64,
        offered: u64,
    },
    UnknownPackage(String),
    UnknownPublisher(String),
    PublisherRevoked(String),
    /// The package bytes do not hash to what the index says.
    HashMismatch,
}

#[derive(Debug, Deserialize)]
struct Envelope {
    payload: String,
    /// Hex Ed25519 signature over the payload bytes.
    sig: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Publisher {
    pub id: String,
    /// Hex Ed25519 public key (32 bytes).
    pub key: String,
    #[serde(default)]
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PackageEntry {
    pub name: String,
    pub version: String,
    pub publisher: String,
    /// Hex SHA-256 of the package bytes.
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Index {
    pub version: u64,
    pub expires_ms: u64,
    pub publishers: Vec<Publisher>,
    pub packages: Vec<PackageEntry>,
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn verify(key_hex: &str, message: &[u8], sig_hex: &str) -> Result<(), RegistryError> {
    let key = unhex(key_hex).ok_or(RegistryError::Malformed)?;
    let sig = unhex(sig_hex).ok_or(RegistryError::Malformed)?;
    UnparsedPublicKey::new(&ED25519, key)
        .verify(message, &sig)
        .map_err(|_| RegistryError::BadSignature)
}

/// Verifies a signed index against the registry's root key and returns it. `last_seen_version` is the newest index
/// version this node accepted before (0 for none); anything older is refused.
pub fn verify_index(
    signed: &str,
    root_key_hex: &str,
    now_ms: u64,
    last_seen_version: u64,
) -> Result<Index, RegistryError> {
    let env: Envelope = serde_json::from_str(signed).map_err(|_| RegistryError::Malformed)?;
    verify(root_key_hex, env.payload.as_bytes(), &env.sig)?;
    let index: Index = serde_json::from_str(&env.payload).map_err(|_| RegistryError::Malformed)?;
    if now_ms >= index.expires_ms {
        return Err(RegistryError::Expired);
    }
    if index.version < last_seen_version {
        return Err(RegistryError::Rollback {
            seen: last_seen_version,
            offered: index.version,
        });
    }
    Ok(index)
}

impl Index {
    /// Checks a downloaded package against its index entry and its publisher's signature. Returns the entry.
    pub fn verify_package(
        &self,
        name: &str,
        bytes: &[u8],
        signature_hex: &str,
    ) -> Result<&PackageEntry, RegistryError> {
        let entry = self
            .packages
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| RegistryError::UnknownPackage(name.to_string()))?;
        let publisher = self
            .publishers
            .iter()
            .find(|p| p.id == entry.publisher)
            .ok_or_else(|| RegistryError::UnknownPublisher(entry.publisher.clone()))?;
        if publisher.revoked {
            return Err(RegistryError::PublisherRevoked(publisher.id.clone()));
        }
        let digest: String = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if digest != entry.sha256.to_ascii_lowercase() {
            return Err(RegistryError::HashMismatch);
        }
        verify(&publisher.key, bytes, signature_hex)?;
        Ok(entry)
    }

    /// Publishers by id, for the user-facing "who published this" step.
    pub fn publishers(&self) -> BTreeMap<&str, &Publisher> {
        self.publishers.iter().map(|p| (p.id.as_str(), p)).collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    struct Key {
        pair: Ed25519KeyPair,
    }

    impl Key {
        fn new() -> Self {
            let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
            Self {
                pair: Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap(),
            }
        }
        fn public_hex(&self) -> String {
            hex(self.pair.public_key().as_ref())
        }
        fn sign_hex(&self, m: &[u8]) -> String {
            hex(self.pair.sign(m).as_ref())
        }
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn sha(b: &[u8]) -> String {
        hex(&Sha256::digest(b))
    }

    fn signed_index(
        root: &Key,
        version: u64,
        expires: u64,
        publisher: &Key,
        revoked: bool,
        pkg: &[u8],
    ) -> String {
        let payload = serde_json::json!({
            "version": version, "expires_ms": expires,
            "publishers": [{"id": "official", "key": publisher.public_hex(), "revoked": revoked}],
            "packages": [{"name": "demo-adapter", "version": "1.0", "publisher": "official", "sha256": sha(pkg)}],
        })
        .to_string();
        serde_json::json!({"payload": payload, "sig": root.sign_hex(payload.as_bytes())})
            .to_string()
    }

    #[test]
    fn a_good_index_and_package_verify() {
        let (root, publisher) = (Key::new(), Key::new());
        let pkg = b"adapter bytes";
        let idx = verify_index(
            &signed_index(&root, 5, 1000, &publisher, false, pkg),
            &root.public_hex(),
            10,
            3,
        )
        .unwrap();
        let entry = idx
            .verify_package("demo-adapter", pkg, &publisher.sign_hex(pkg))
            .unwrap();
        assert_eq!(entry.version, "1.0");
        assert_eq!(idx.publishers()["official"].id, "official");
    }

    #[test]
    fn an_index_not_signed_by_the_pinned_root_is_refused() {
        let (root, other, publisher) = (Key::new(), Key::new(), Key::new());
        let signed = signed_index(&other, 1, 1000, &publisher, false, b"x");
        assert_eq!(
            verify_index(&signed, &root.public_hex(), 0, 0),
            Err(RegistryError::BadSignature)
        );
        // Tampering with the payload after signing breaks the signature.
        let good = signed_index(&root, 1, 1000, &publisher, false, b"x");
        let tampered = good.replace("demo-adapter", "evil-adapter");
        assert!(matches!(
            verify_index(&tampered, &root.public_hex(), 0, 0),
            Err(RegistryError::BadSignature | RegistryError::Malformed)
        ));
        assert_eq!(
            verify_index("not json", &root.public_hex(), 0, 0),
            Err(RegistryError::Malformed)
        );
    }

    #[test]
    fn expired_and_rolled_back_indexes_are_refused() {
        let (root, publisher) = (Key::new(), Key::new());
        let s = signed_index(&root, 5, 1000, &publisher, false, b"x");
        assert_eq!(
            verify_index(&s, &root.public_hex(), 1000, 0),
            Err(RegistryError::Expired)
        );
        assert_eq!(
            verify_index(&s, &root.public_hex(), 10, 6),
            Err(RegistryError::Rollback {
                seen: 6,
                offered: 5
            })
        );
        assert!(
            verify_index(&s, &root.public_hex(), 10, 5).is_ok(),
            "the same version is fine"
        );
    }

    #[test]
    fn a_package_must_match_its_hash_and_its_publishers_signature() {
        let (root, publisher, thief) = (Key::new(), Key::new(), Key::new());
        let pkg = b"adapter bytes";
        let idx = verify_index(
            &signed_index(&root, 1, 1000, &publisher, false, pkg),
            &root.public_hex(),
            0,
            0,
        )
        .unwrap();
        // Different bytes (a swapped package): hash mismatch even with a valid signature over them.
        let evil = b"evil bytes";
        assert_eq!(
            idx.verify_package("demo-adapter", evil, &publisher.sign_hex(evil)),
            Err(RegistryError::HashMismatch)
        );
        // Right bytes, signed by someone else.
        assert_eq!(
            idx.verify_package("demo-adapter", pkg, &thief.sign_hex(pkg)),
            Err(RegistryError::BadSignature)
        );
        assert_eq!(
            idx.verify_package("demo-adapter", pkg, "zz"),
            Err(RegistryError::Malformed)
        );
        assert_eq!(
            idx.verify_package("nope", pkg, &publisher.sign_hex(pkg)),
            Err(RegistryError::UnknownPackage("nope".into()))
        );
    }

    #[test]
    fn a_revoked_publisher_is_refused_even_with_a_valid_signature() {
        let (root, publisher) = (Key::new(), Key::new());
        let pkg = b"adapter bytes";
        let idx = verify_index(
            &signed_index(&root, 1, 1000, &publisher, true, pkg),
            &root.public_hex(),
            0,
            0,
        )
        .unwrap();
        assert_eq!(
            idx.verify_package("demo-adapter", pkg, &publisher.sign_hex(pkg)),
            Err(RegistryError::PublisherRevoked("official".into()))
        );
    }

    #[test]
    fn unknown_publishers_are_refused() {
        let (root, publisher) = (Key::new(), Key::new());
        let payload = serde_json::json!({
            "version": 1, "expires_ms": 1000, "publishers": [],
            "packages": [{"name": "p", "version": "1", "publisher": "ghost", "sha256": sha(b"x")}],
        })
        .to_string();
        let signed =
            serde_json::json!({"payload": payload, "sig": root.sign_hex(payload.as_bytes())})
                .to_string();
        let idx = verify_index(&signed, &root.public_hex(), 0, 0).unwrap();
        assert_eq!(
            idx.verify_package("p", b"x", &publisher.sign_hex(b"x")),
            Err(RegistryError::UnknownPublisher("ghost".into()))
        );
    }
}
