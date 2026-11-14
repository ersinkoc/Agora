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

/// The bytes a device signs to decide an agent's request to start another (ADR-0057, `agora-node::approvals::start_signed_message`, which
/// a test compares against): the decision, the grant and a hash of the request exactly as it was shown.
pub fn start_message(
    node_id: &str,
    approval_id: &str,
    approved: bool,
    helper_grant: u32,
    helpers_may_start: bool,
    detail: &str,
) -> Vec<u8> {
    let hash = hex(ring::digest::digest(&ring::digest::SHA256, detail.as_bytes()).as_ref());
    let mut m = b"agora-approval-start-v1".to_vec();
    for part in [
        node_id,
        approval_id,
        if approved { "1" } else { "0" },
        &helper_grant.to_string(),
        if helpers_may_start { "1" } else { "0" },
        &hash,
    ] {
        m.push(0);
        m.extend_from_slice(part.as_bytes());
    }
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

/// The exact bytes a device signs for a folder action (ADR-0052 / ADR-0053: `inspect`, `list`, `launch`; `agora-node::directory::
/// signed_message`, which a test compares against): a domain tag, then the node, the operation, the path, the agent kind, the expected
/// project, the option and identity-change flags, the nonce and the expiry in decimal milliseconds, NUL-separated.
#[allow(clippy::too_many_arguments)] // the fields the node binds, each on its own
pub fn directory_message(
    node: &str,
    operation: &str,
    path: &str,
    kind: &str,
    project: &str,
    option: bool,
    identity_change: bool,
    nonce: &str,
    expires_at_ms: u64,
) -> Vec<u8> {
    let mut m = b"directory-action-v1".to_vec();
    for part in [
        node,
        operation,
        path,
        kind,
        project,
        if option { "1" } else { "0" },
        if identity_change { "1" } else { "0" },
        nonce,
    ] {
        m.push(0);
        m.extend_from_slice(part.as_bytes());
    }
    m.push(0);
    m.extend_from_slice(expires_at_ms.to_string().as_bytes());
    m
}

/// ADR-0059: a list as signed: its count, then `:<byte length>:<item>` per item (the node's `agora_node::directory::signed_list`).
fn signed_list(items: &[String]) -> String {
    let mut out = items.len().to_string();
    for i in items {
        out.push_str(&format!(":{}:{i}", i.len()));
    }
    out
}

/// ADR-0059: what a device signs for a launch that chose launch options: the `directory-action-v1` launch fields under the domain
/// `directory-action-v3`, then the chosen ids and the ids the user confirmed, as two length-prefixed lists, before the nonce. Checked
/// against `schema/fixtures/signing/directory-action-v3.json`, as the node and the web window are.
#[allow(clippy::too_many_arguments)] // the fields the node binds, each on its own
pub fn launch_message(
    node: &str,
    path: &str,
    kind: &str,
    project: &str,
    separate: bool,
    identity_change: bool,
    launch_options: &[String],
    confirmed: &[String],
    nonce: &str,
    expires_at_ms: u64,
) -> Vec<u8> {
    let (options, confirmed) = (signed_list(launch_options), signed_list(confirmed));
    let mut m = b"directory-action-v3".to_vec();
    for part in [
        node,
        "launch",
        path,
        kind,
        project,
        if separate { "1" } else { "0" },
        if identity_change { "1" } else { "0" },
        options.as_str(),
        confirmed.as_str(),
        nonce,
    ] {
        m.push(0);
        m.extend_from_slice(part.as_bytes());
    }
    m.push(0);
    m.extend_from_slice(expires_at_ms.to_string().as_bytes());
    m
}

/// How long a signed folder action stays valid (the node refuses more than five minutes ahead).
const FOLDER_ACTION_TTL_MS: u64 = 2 * 60 * 1000;

impl Device {
    /// This device's authorization for one folder action, with a fresh nonce, valid for two minutes from `now_ms`.
    #[allow(clippy::too_many_arguments)] // the fields the node binds, each on its own
    pub fn authorize(
        &self,
        node: &str,
        operation: &str,
        path: &str,
        kind: &str,
        project: &str,
        option: bool,
        identity_change: bool,
        now_ms: u64,
    ) -> Option<agora_protocol::v1::DirectoryAuthorization> {
        use ring::rand::SecureRandom;
        let mut b = [0u8; 16];
        SystemRandom::new().fill(&mut b).ok()?;
        let nonce = hex(&b);
        let expires_at_ms = now_ms + FOLDER_ACTION_TTL_MS;
        let message = directory_message(
            node,
            operation,
            path,
            kind,
            project,
            option,
            identity_change,
            &nonce,
            expires_at_ms,
        );
        Some(agora_protocol::v1::DirectoryAuthorization {
            device_id: self.id.clone(),
            nonce,
            expires_at_ms,
            signature: self.sign(&message)?,
        })
    }

    /// [`Self::authorize`] for a launch: with launch options chosen (ADR-0059) the signature covers them too.
    #[allow(clippy::too_many_arguments)] // the fields the node binds, each on its own
    pub fn authorize_launch(
        &self,
        node: &str,
        path: &str,
        kind: &str,
        project: &str,
        separate: bool,
        identity_change: bool,
        launch_options: &[String],
        confirmed: &[String],
        now_ms: u64,
    ) -> Option<agora_protocol::v1::DirectoryAuthorization> {
        if launch_options.is_empty() && confirmed.is_empty() {
            return self.authorize(
                node,
                "launch",
                path,
                kind,
                project,
                separate,
                identity_change,
                now_ms,
            );
        }
        use ring::rand::SecureRandom;
        let mut b = [0u8; 16];
        SystemRandom::new().fill(&mut b).ok()?;
        let nonce = hex(&b);
        let expires_at_ms = now_ms + FOLDER_ACTION_TTL_MS;
        let message = launch_message(
            node,
            path,
            kind,
            project,
            separate,
            identity_change,
            launch_options,
            confirmed,
            &nonce,
            expires_at_ms,
        );
        Some(agora_protocol::v1::DirectoryAuthorization {
            device_id: self.id.clone(),
            nonce,
            expires_at_ms,
            signature: self.sign(&message)?,
        })
    }
}

fn field(out: &mut Vec<u8>, x: &[u8]) {
    out.extend_from_slice(x.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(x);
}

/// The bytes a device signs to endorse another (ADR-0061, `agora-node::device_trust::endorse_message`; vectors in
/// `schema/fixtures/signing/device-trust-v1.json`).
pub fn endorse_message(e: &agora_protocol::v1::DeviceEndorsement) -> Vec<u8> {
    let mut m = b"agora-device-endorse-v1\0".to_vec();
    field(&mut m, e.device_id.as_bytes());
    field(&mut m, hex(&e.public_key).as_bytes());
    field(&mut m, e.name.as_bytes());
    field(&mut m, e.nodes.len().to_string().as_bytes());
    for n in &e.nodes {
        field(&mut m, n.as_bytes());
    }
    field(&mut m, e.issued_at_ms.to_string().as_bytes());
    field(&mut m, e.expires_at_ms.to_string().as_bytes());
    field(&mut m, e.endorser_device_id.as_bytes());
    m
}

/// The bytes a device signs to revoke one (ADR-0061).
pub fn revoke_message(r: &agora_protocol::v1::DeviceRevocation) -> Vec<u8> {
    let mut m = b"agora-device-revoke-v1\0".to_vec();
    field(&mut m, r.device_id.as_bytes());
    field(&mut m, r.issued_at_ms.to_string().as_bytes());
    field(&mut m, r.revoker_device_id.as_bytes());
    m
}

/// The bytes a device signs to register itself (ADR-0061 proof of possession; `agora-node::device_trust::register_message`).
pub fn register_message(
    device_id: &str,
    name: &str,
    public_key: &[u8],
    signed_at_ms: u64,
) -> Vec<u8> {
    let mut m = b"agora-device-register-v1\0".to_vec();
    field(&mut m, device_id.as_bytes());
    field(&mut m, name.as_bytes());
    field(&mut m, hex(public_key).as_bytes());
    field(&mut m, signed_at_ms.to_string().as_bytes());
    m
}

/// The bytes a device signs to ask another device of its person to approve it.
pub fn ask_message(device_id: &str, signed_at_ms: u64) -> Vec<u8> {
    let mut m = b"agora-device-ask-v1\0".to_vec();
    field(&mut m, device_id.as_bytes());
    field(&mut m, signed_at_ms.to_string().as_bytes());
    m
}

/// What people compare (ADR-0061): the first 12 bytes (96 bits) of the SHA-256 of the key, in groups of 4 hex digits joined by `-`.
pub fn fingerprint(public_key: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, public_key)
        .as_ref()
        .get(..12)
        .unwrap_or_default()
        .chunks(2)
        .map(hex)
        .collect::<Vec<_>>()
        .join("-")
}

/// How long an endorsement this terminal signs lasts (ADR-0061: 90 days; the machine renews it by use).
pub const ENDORSEMENT_TTL_MS: u64 = 90 * 24 * 60 * 60 * 1000;

impl Device {
    /// The 65-byte public key.
    pub fn public_key(&self) -> Vec<u8> {
        self.pair.public_key().as_ref().to_vec()
    }

    /// This device registering itself under `name`, signed at `now_ms` with its own key.
    pub fn registration(
        &self,
        name: &str,
        now_ms: u64,
    ) -> Option<agora_protocol::v1::DeviceRegister> {
        let public_key = self.public_key();
        Some(agora_protocol::v1::DeviceRegister {
            signature: self.sign(&register_message(&self.id, name, &public_key, now_ms))?,
            device_id: self.id.clone(),
            name: name.to_string(),
            public_key,
            signed_at_ms: now_ms,
        })
    }

    /// This device asking another of its person to approve it, signed at `now_ms`.
    pub fn ask(&self, now_ms: u64) -> Option<agora_protocol::v1::DeviceAskApproval> {
        Some(agora_protocol::v1::DeviceAskApproval {
            device_id: self.id.clone(),
            signed_at_ms: now_ms,
            signature: self.sign(&ask_message(&self.id, now_ms))?,
        })
    }

    /// This device's fingerprint, as every client shows it.
    pub fn fingerprint(&self) -> String {
        fingerprint(self.pair.public_key().as_ref())
    }

    /// This device vouches for `target` (another device of the same person) on `nodes` (empty: every machine) for 90 days from
    /// `now_ms` (ADR-0061).
    pub fn endorse(
        &self,
        target: &agora_protocol::v1::DeviceInfo,
        nodes: &[String],
        now_ms: u64,
    ) -> Option<agora_protocol::v1::DeviceEndorsement> {
        let mut e = agora_protocol::v1::DeviceEndorsement {
            device_id: target.device_id.clone(),
            public_key: target.public_key.clone(),
            name: target.name.clone(),
            nodes: nodes.to_vec(),
            issued_at_ms: now_ms,
            expires_at_ms: now_ms + ENDORSEMENT_TTL_MS,
            endorser_device_id: self.id.clone(),
            signature: Vec::new(),
        };
        e.signature = self.sign(&endorse_message(&e))?;
        Some(e)
    }
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
    fn the_start_message_is_the_one_the_node_rebuilds() {
        // The vector the node and the web client test too: non-ASCII, a raw line break and an emoji, hashed byte for byte.
        let detail = "{\"prompt\":\"Grüße\nçalış 🚀\"}";
        let m = start_message("node-1", "ap-1", true, 3, false, detail);
        assert!(m.ends_with(b"2705e6e894314e40e3938d6e7780386d393351c5d33a5826ed13948b56d8c076"));
        assert_eq!(
            m,
            agora_node::approvals::start_signed_message("node-1", "ap-1", true, 3, false, detail)
        );
        for (approved, grant, nested) in [(true, 0, false), (true, 3, true), (false, 0, false)] {
            assert_eq!(
                start_message("laptop", "7", approved, grant, nested, "{\"prompt\":\"x\"}"),
                agora_node::approvals::start_signed_message(
                    "laptop",
                    "7",
                    approved,
                    grant,
                    nested,
                    "{\"prompt\":\"x\"}"
                )
            );
        }
    }

    #[test]
    fn the_folder_action_message_is_the_one_the_node_rebuilds() {
        for (op, kind, project, option, change) in [
            ("list", "", "", false, false),
            ("inspect", "", "", true, false),
            ("launch", "claude-code", "id:p", false, true),
        ] {
            assert_eq!(
                directory_message(
                    "laptop",
                    op,
                    "/Users/me/code",
                    kind,
                    project,
                    option,
                    change,
                    "00ff",
                    1_700_000_000_000
                ),
                agora_node::directory::signed_message(
                    "laptop",
                    op,
                    "/Users/me/code",
                    kind,
                    project,
                    option,
                    change,
                    "00ff",
                    1_700_000_000_000
                )
            );
        }
    }

    #[test]
    fn a_launch_with_options_signs_the_shared_vectors() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/fixtures/signing/directory-action-v3.json");
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let list = |x: &serde_json::Value| -> Vec<String> {
            x.as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_string())
                .collect()
        };
        for c in v["cases"].as_array().unwrap() {
            let m = launch_message(
                c["node"].as_str().unwrap(),
                c["path"].as_str().unwrap(),
                c["kind"].as_str().unwrap(),
                c["project"].as_str().unwrap(),
                c["separate"].as_bool().unwrap(),
                c["identity_change"].as_bool().unwrap(),
                &list(&c["options"]),
                &list(&c["confirmed"]),
                c["nonce"].as_str().unwrap(),
                c["expires"].as_u64().unwrap(),
            );
            assert_eq!(hex(&m), c["hex"].as_str().unwrap(), "{}", c["name"]);
        }
        let one = vec!["a,b".to_string()];
        let two = vec!["a".to_string(), "b".to_string()];
        assert_ne!(
            launch_message("n", "/p", "k", "id", false, false, &one, &[], "00", 1),
            launch_message("n", "/p", "k", "id", false, false, &two, &[], "00", 1)
        );
        let (d, _) = Device::generate().unwrap();
        assert!(d
            .authorize_launch(
                "laptop",
                "/w",
                "codex",
                "id:p",
                false,
                false,
                &one,
                &[],
                1_000
            )
            .is_some());
    }

    #[test]
    fn an_authorization_has_a_fresh_32_digit_nonce_and_a_short_life() {
        let (d, _) = Device::generate().unwrap();
        let a = d
            .authorize("laptop", "list", "", "", "", false, false, 1_000)
            .unwrap();
        let b = d
            .authorize("laptop", "list", "", "", "", false, false, 1_000)
            .unwrap();
        assert_eq!(a.nonce.len(), 32);
        assert!(a.nonce.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a.nonce, b.nonce);
        assert_eq!(a.expires_at_ms, 1_000 + 120_000);
        assert_eq!(a.device_id, d.id);
        assert_eq!(a.signature.len(), 64);
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

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn endorsements_revocations_and_fingerprints_match_the_shared_vectors_and_the_node() {
        let v: serde_json::Value = serde_json::from_str(include_str!(
            "../../../schema/fixtures/signing/device-trust-v1.json"
        ))
        .unwrap();
        for c in v["endorsements"].as_array().unwrap() {
            let e = agora_protocol::v1::DeviceEndorsement {
                device_id: c["device_id"].as_str().unwrap().into(),
                public_key: unhex(c["public_key"].as_str().unwrap()),
                name: c["name"].as_str().unwrap().into(),
                nodes: c["nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n.as_str().unwrap().to_string())
                    .collect(),
                issued_at_ms: c["issued_at_ms"].as_u64().unwrap(),
                expires_at_ms: c["expires_at_ms"].as_u64().unwrap(),
                endorser_device_id: c["endorser_device_id"].as_str().unwrap().into(),
                signature: Vec::new(),
            };
            assert_eq!(hex(&endorse_message(&e)), c["hex"].as_str().unwrap());
            assert_eq!(
                endorse_message(&e),
                agora_node::device_trust::endorse_message(&e)
            );
        }
        for c in v["revocations"].as_array().unwrap() {
            let r = agora_protocol::v1::DeviceRevocation {
                device_id: c["device_id"].as_str().unwrap().into(),
                issued_at_ms: c["issued_at_ms"].as_u64().unwrap(),
                revoker_device_id: c["revoker_device_id"].as_str().unwrap().into(),
                signature: Vec::new(),
            };
            assert_eq!(hex(&revoke_message(&r)), c["hex"].as_str().unwrap());
        }
        for c in v["registrations"].as_array().unwrap() {
            let m = register_message(
                c["device_id"].as_str().unwrap(),
                c["name"].as_str().unwrap(),
                &unhex(c["public_key"].as_str().unwrap()),
                c["signed_at_ms"].as_u64().unwrap(),
            );
            assert_eq!(hex(&m), c["hex"].as_str().unwrap());
        }
        for c in v["asks"].as_array().unwrap() {
            let m = ask_message(
                c["device_id"].as_str().unwrap(),
                c["signed_at_ms"].as_u64().unwrap(),
            );
            assert_eq!(hex(&m), c["hex"].as_str().unwrap());
        }
        for f in v["fingerprints"].as_array().unwrap() {
            assert_eq!(
                fingerprint(&unhex(f["public_key"].as_str().unwrap())),
                f["fingerprint"].as_str().unwrap()
            );
        }
    }

    #[test]
    fn an_endorsement_this_terminal_signs_is_accepted_by_a_machine_that_trusts_it() {
        let (me, _) = Device::generate().unwrap();
        let (other, _) = Device::generate().unwrap();
        let dir = std::env::temp_dir().join(format!("agora-tui-endorse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("trusted-devices");
        agora_node::device_trust::confirm(&file, &me.id, &me.public_key()).unwrap();
        let target = agora_protocol::v1::DeviceInfo {
            device_id: other.id.clone(),
            name: "Phone".into(),
            public_key: other.public_key(),
            ..Default::default()
        };
        let now = 1_700_000_000_000;
        let e = me.endorse(&target, &[], now).unwrap();
        let trust = agora_node::device_trust::DeviceTrust::new("laptop", &file);
        assert!(trust.endorse(&e, now).is_ok());
        assert!(trust.trusted_ids(now).contains(&other.id));
        assert_eq!(me.fingerprint().len(), 29, "96 bits in groups of 4");
        let r = me.registration("Terminal on box", now).unwrap();
        let info = agora_protocol::v1::DeviceInfo {
            device_id: r.device_id.clone(),
            name: r.name.clone(),
            public_key: r.public_key.clone(),
            signed_at_ms: r.signed_at_ms,
            registration_signature: r.signature.clone(),
            ..Default::default()
        };
        assert!(agora_node::device_trust::registration_verifies(&info));
        assert_eq!(
            ask_message("x", 1),
            b"agora-device-ask-v1\x001:x1:1".to_vec()
        );
        let _ = std::fs::remove_dir_all(dir);
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
