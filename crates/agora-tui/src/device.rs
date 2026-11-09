//! This terminal's signing key (ADR-0043 slice 2, ADR-0028): ECDSA P-256 over SHA-256, the scheme every client uses. The private key
//! stays in a file only its owner can read (the binary writes it); the hub never sees it, and a machine only accepts it after its
//! owner ran `agora node trust` with the public key.

use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};

pub struct Device {
    /// The name the machine's trusted-devices file knows this key by.
    pub id: String,
    pair: EcdsaKeyPair,
}

impl Device {
    /// A new key: the device and its PKCS#8 bytes to store.
    pub fn generate() -> Option<(Device, Vec<u8>)> {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).ok()?;
        let bytes = pkcs8.as_ref().to_vec();
        let device = Device::from_pkcs8(&bytes)?;
        Some((device, bytes))
    }

    /// The device a stored key belongs to. Its id is `tui-` and the first three bytes of its public key.
    pub fn from_pkcs8(bytes: &[u8]) -> Option<Device> {
        let rng = SystemRandom::new();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, bytes, &rng).ok()?;
        let key = pair.public_key().as_ref();
        let id = format!("tui-{}", hex(key.get(1..4)?));
        Some(Device { id, pair })
    }

    /// The 65-byte uncompressed point as 130 hex digits: what `agora node trust` takes.
    pub fn public_key_hex(&self) -> String {
        hex(self.pair.public_key().as_ref())
    }

    /// A fixed 64-byte (r || s) signature.
    pub fn sign(&self, message: &[u8]) -> Option<Vec<u8>> {
        let rng = SystemRandom::new();
        self.pair
            .sign(&rng, message)
            .ok()
            .map(|s| s.as_ref().to_vec())
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The exact bytes a device signs to answer an approval (`agora-node::approvals::signed_message`, which a test compares against).
pub fn approval_message(node_id: &str, approval_id: &str, approved: bool) -> Vec<u8> {
    let mut m = b"agora-approval-v1".to_vec();
    for part in [node_id.as_bytes(), approval_id.as_bytes()] {
        m.push(0);
        m.extend_from_slice(part);
    }
    m.push(0);
    m.push(u8::from(approved));
    m
}

/// The exact bytes a device signs for a grant (ADR-0041): a domain tag, then node id, grant id, scope, the commands joined by commas and
/// the expiry in decimal milliseconds, NUL-separated.
pub fn grant_message(
    node_id: &str,
    grant_id: &str,
    scope: &str,
    commands: &[String],
    expires_at_ms: u64,
) -> Vec<u8> {
    let mut m = b"agora-grant-v1".to_vec();
    for part in [
        node_id,
        grant_id,
        scope,
        &commands.join(","),
        &expires_at_ms.to_string(),
    ] {
        m.push(0);
        m.extend_from_slice(part.as_bytes());
    }
    m
}

/// A fresh grant id.
pub fn new_grant_id() -> Option<String> {
    use ring::rand::SecureRandom;
    let mut b = [0u8; 8];
    SystemRandom::new().fill(&mut b).ok()?;
    Some(format!("gr-{}", hex(&b)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn the_message_is_the_one_the_node_rebuilds() {
        for approved in [true, false] {
            assert_eq!(
                approval_message("laptop", "7", approved),
                agora_node::approvals::signed_message("laptop", "7", approved)
            );
        }
    }

    #[test]
    fn the_grant_message_is_the_one_the_node_rebuilds() {
        let commands = vec!["agent.stop".to_string(), "agent.prompt".to_string()];
        assert_eq!(
            grant_message(
                "laptop",
                "gr-1",
                "agent:agent-1",
                &commands,
                1_700_000_000_000
            ),
            agora_node::grants::signed_message(
                "laptop",
                "gr-1",
                "agent:agent-1",
                &commands,
                1_700_000_000_000
            )
        );
        let id = new_grant_id().unwrap();
        assert!(id.starts_with("gr-") && id.len() == 19, "{id}");
    }

    #[test]
    fn a_stored_key_comes_back_as_the_same_device_and_a_node_accepts_its_signature() {
        let (d, bytes) = Device::generate().unwrap();
        let again = Device::from_pkcs8(&bytes).unwrap();
        assert_eq!(d.id, again.id);
        assert_eq!(d.public_key_hex(), again.public_key_hex());
        assert_eq!(d.public_key_hex().len(), 130);
        // The node's own verifier takes the id and the public key as `agora node trust` gives them.
        let mut v = agora_node::approvals::P256Verifier::new();
        let key: Vec<u8> = (0..65)
            .map(|i| u8::from_str_radix(&d.public_key_hex()[i * 2..i * 2 + 2], 16).unwrap())
            .collect();
        assert!(v.trust(&d.id, &key));
        let msg = approval_message("laptop", "7", true);
        let sig = d.sign(&msg).unwrap();
        assert_eq!(sig.len(), 64);
        use agora_node::approvals::Verifier;
        assert!(v.verify(&d.id, &msg, &sig));
        assert!(!v.verify(&d.id, &approval_message("laptop", "7", false), &sig));
    }
}
