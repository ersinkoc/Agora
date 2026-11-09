//! Approvals on the node (ADR-0020, threat T1): sensitive actions wait here until a user's device signs a decision
//! and the node verifies that signature itself. The hub only relays; a compromised hub can neither approve nor
//! widen the policy. Mesh content and agent output never reach this decision (T4).
//!
//! What is enforced:
//! - the signature must verify under a key the node already trusts for that device id (unknown signer: refused);
//! - the signed message binds this node, this approval and the verdict, so a decision cannot be replayed on
//!   another node, for another approval, or flipped from deny to allow;
//! - approval ids are random and single use: a decision applies once, then the id is spent (replay refused);
//! - requests expire: a decision that arrives late is refused and the agent is told it expired;
//! - policy decides which actions need approval at all (`Policy::requires`), default allow only for what the user
//!   did not list.
//!
//! The signature scheme is behind [`Verifier`]: choosing and implementing it (ECDSA P-256 is the candidate, since
//! WebAuthn, WebCrypto and secure enclaves all have it) is a separate decision.

use std::collections::{BTreeMap, VecDeque};

use agora_protocol::v1::ApprovalResolve;

/// How long a request waits for a decision by default.
pub const DEFAULT_TTL_MS: u64 = 10 * 60 * 1000;
/// Spent ids remembered to refuse replays.
const SPENT_MEMORY: usize = 4096;

/// Checks a device's signature over a message. `false` for an unknown device as well as a bad signature, so callers
/// cannot tell the two apart from the outside.
pub trait Verifier {
    fn verify(&self, device_id: &str, message: &[u8], signature: &[u8]) -> bool;
}

/// The exact bytes a device signs: a domain tag, then node id, approval id and verdict, NUL-separated.
pub fn signed_message(node_id: &str, approval_id: &str, approved: bool) -> Vec<u8> {
    let mut m = b"agora-approval-v1".to_vec();
    for part in [node_id.as_bytes(), approval_id.as_bytes()] {
        m.push(0);
        m.extend_from_slice(part);
    }
    m.push(0);
    m.push(u8::from(approved));
    m
}

/// Which actions need a person. Patterns are exact action names, or a prefix ending in `*`; `*` alone is everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    pub approval_required: Vec<String>,
}

impl Policy {
    pub fn requires(&self, action: &str) -> bool {
        self.approval_required
            .iter()
            .any(|p| match p.strip_suffix('*') {
                Some(prefix) => action.starts_with(prefix),
                None => p == action,
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub id: String,
    pub agent_id: String,
    pub action: String,
    pub detail: String,
    pub requested_ms: u64,
    pub expires_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Approved,
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalError {
    /// No such pending request: never existed, or already answered (indistinguishable on purpose).
    Unknown,
    /// The request ran out of time before a decision arrived.
    Expired,
    /// Unknown device or a signature that does not verify.
    BadSignature,
}

pub struct Approvals<V: Verifier> {
    node_id: String,
    verifier: V,
    pending: BTreeMap<String, Pending>,
    spent: VecDeque<String>,
}

fn random_id() -> String {
    let mut b = [0u8; 16];
    // A node without system randomness cannot mint safe ids; refusing to run beats predictable approvals.
    if getrandom::getrandom(&mut b).is_err() {
        return String::new();
    }
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("ap-{hex}")
}

impl<V: Verifier> Approvals<V> {
    pub fn new(node_id: &str, verifier: V) -> Self {
        Self {
            node_id: node_id.to_string(),
            verifier,
            pending: BTreeMap::new(),
            spent: VecDeque::new(),
        }
    }

    /// Replaces the verifier (when trusted device keys are loaded or changed).
    pub fn set_verifier(&mut self, verifier: V) {
        self.verifier = verifier;
    }

    /// Registers a request and returns its id, or `None` if no safe id could be made (the action must then be
    /// refused). `ttl_ms` of 0 uses the default.
    pub fn request(
        &mut self,
        agent_id: &str,
        action: &str,
        detail: &str,
        now_ms: u64,
        ttl_ms: u64,
    ) -> Option<Pending> {
        let id = random_id();
        if id.is_empty() {
            return None;
        }
        let ttl = if ttl_ms == 0 { DEFAULT_TTL_MS } else { ttl_ms };
        let p = Pending {
            id: id.clone(),
            agent_id: agent_id.to_string(),
            action: action.to_string(),
            detail: detail.to_string(),
            requested_ms: now_ms,
            expires_ms: now_ms.saturating_add(ttl),
        };
        self.pending.insert(id, p.clone());
        Some(p)
    }

    pub fn pending(&self) -> impl Iterator<Item = &Pending> {
        self.pending.values()
    }

    /// Applies a signed decision. On success the request is removed and its id spent.
    pub fn resolve(
        &mut self,
        r: &ApprovalResolve,
        now_ms: u64,
    ) -> Result<(Pending, Verdict), ApprovalError> {
        let Some(p) = self.pending.get(&r.approval_id) else {
            return Err(ApprovalError::Unknown);
        };
        // Verify before anything changes, and before saying whether the request expired: a peer without a trusted
        // key cannot learn the state of a request it can name (ids are 128 random bits, so it cannot guess one).
        let msg = signed_message(&self.node_id, &r.approval_id, r.approved);
        if !self
            .verifier
            .verify(&r.signer_device_id, &msg, &r.signature)
        {
            return Err(ApprovalError::BadSignature);
        }
        if now_ms >= p.expires_ms {
            let p = self.pending.remove(&r.approval_id);
            self.spend(&r.approval_id);
            drop(p);
            return Err(ApprovalError::Expired);
        }
        let p = self
            .pending
            .remove(&r.approval_id)
            .ok_or(ApprovalError::Unknown)?;
        self.spend(&r.approval_id);
        let verdict = if r.approved {
            Verdict::Approved
        } else {
            Verdict::Denied
        };
        Ok((p, verdict))
    }

    /// Removes and returns requests whose time ran out; the caller tells each agent it expired.
    pub fn expire(&mut self, now_ms: u64) -> Vec<Pending> {
        let ids: Vec<String> = self
            .pending
            .values()
            .filter(|p| now_ms >= p.expires_ms)
            .map(|p| p.id.clone())
            .collect();
        ids.into_iter()
            .filter_map(|id| {
                self.spend(&id);
                self.pending.remove(&id)
            })
            .collect()
    }

    /// Withdraws every request of an agent that stopped; nothing is left waiting on a dead agent.
    pub fn cancel_agent(&mut self, agent_id: &str) -> Vec<Pending> {
        let ids: Vec<String> = self
            .pending
            .values()
            .filter(|p| p.agent_id == agent_id)
            .map(|p| p.id.clone())
            .collect();
        ids.into_iter()
            .filter_map(|id| {
                self.spend(&id);
                self.pending.remove(&id)
            })
            .collect()
    }

    fn spend(&mut self, id: &str) {
        self.spent.push_back(id.to_string());
        while self.spent.len() > SPENT_MEMORY {
            self.spent.pop_front();
        }
    }

    /// Was this id answered, expired or cancelled before? (Replays are refused as `Unknown` by `resolve`, because a
    /// spent id is no longer pending; this is for audit.)
    pub fn was_spent(&self, id: &str) -> bool {
        self.spent.iter().any(|s| s == id)
    }
}

impl Verifier for Box<dyn Verifier + Send> {
    fn verify(&self, device_id: &str, message: &[u8], signature: &[u8]) -> bool {
        (**self).verify(device_id, message, signature)
    }
}

/// Verifies ECDSA P-256/SHA-256 signatures (ADR-0028) against the public keys of trusted devices. Keys are uncompressed
/// SEC1 points (65 bytes); signatures are fixed 64 bytes (r || s). An unknown device never verifies.
#[derive(Default)]
pub struct P256Verifier {
    keys: std::collections::HashMap<String, Vec<u8>>,
}

impl P256Verifier {
    pub fn new() -> Self {
        Self::default()
    }

    /// Trusts `device_id` with this public key. Returns false (and trusts nothing) if it is not a valid point encoding.
    pub fn trust(&mut self, device_id: &str, public_key: &[u8]) -> bool {
        if public_key.len() != 65 || public_key[0] != 4 {
            return false;
        }
        self.keys.insert(device_id.to_string(), public_key.to_vec());
        true
    }

    pub fn revoke(&mut self, device_id: &str) {
        self.keys.remove(device_id);
    }
}

/// Name of the file in the node's data directory that lists the devices whose signatures this node accepts.
pub const TRUSTED_DEVICES_FILE: &str = "trusted-devices";

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn valid_device_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

impl P256Verifier {
    /// Reads the trusted-devices file: one `<device id> <public key as 130 hex digits>` per line; blank lines and `#`
    /// comments are skipped. A missing file trusts nothing. Lines that do not parse are skipped (the count of those
    /// is returned) rather than trusted: a typo must never widen trust.
    pub fn load(path: &std::path::Path) -> (Self, usize) {
        let mut v = Self::new();
        let mut skipped = 0;
        let Ok(text) = std::fs::read_to_string(path) else {
            return (v, 0);
        };
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let ok = line
                .split_once(char::is_whitespace)
                .and_then(|(id, key)| {
                    let key = from_hex(key.trim())?;
                    (valid_device_id(id) && v.trust(id, &key)).then_some(())
                })
                .is_some();
            if !ok {
                skipped += 1;
            }
        }
        (v, skipped)
    }
}

/// Adds (or replaces) a device in the trusted-devices file. This is the node owner's act on the node itself: the
/// hub has no way to call it, which is what keeps approvals from being widened remotely (ADR-0020).
pub fn trust_device(path: &std::path::Path, device_id: &str, key_hex: &str) -> Result<(), String> {
    let key = from_hex(key_hex).ok_or("the key is not hexadecimal")?;
    if !valid_device_id(device_id) {
        return Err("the device id may contain only letters, digits, '.', '_' and '-'".into());
    }
    if P256Verifier::new().trust(device_id, &key) {
        let old = std::fs::read_to_string(path).unwrap_or_default();
        let mut out: String = old
            .lines()
            .filter(|l| l.split_whitespace().next() != Some(device_id))
            .map(|l| format!("{l}\n"))
            .collect();
        out.push_str(&format!("{device_id} {}\n", key_hex.to_ascii_lowercase()));
        return std::fs::write(path, out).map_err(|e| e.to_string());
    }
    Err("the key is not an uncompressed P-256 point (65 bytes)".into())
}

impl Verifier for P256Verifier {
    fn verify(&self, device_id: &str, message: &[u8], signature: &[u8]) -> bool {
        let Some(key) = self.keys.get(device_id) else {
            return false;
        };
        ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_FIXED, key)
            .verify(message, signature)
            .is_ok()
    }
}

/// Trusts no device: every decision is refused. The safe default until the node has trusted device keys (and the
/// signature scheme exists): requests wait, expire, and the agent is told they were denied.
pub struct DenyAll;

impl Verifier for DenyAll {
    fn verify(&self, _device_id: &str, _message: &[u8], _signature: &[u8]) -> bool {
        false
    }
}

/// The node's approvals queue plus the routes back to the adapters that asked. The adapter picked its own approval
/// id; the node mints a random one (the one users and devices see and sign) and remembers the pairing.
pub struct NodeApprovals {
    queue: std::sync::Mutex<Approvals<Box<dyn Verifier + Send>>>,
    routes: std::sync::Mutex<std::collections::HashMap<String, (agora_core::AgentId, String)>>,
    /// The trusted-devices file and its modification time when it was last read.
    /// The file and what was last read from it. The content is compared, not only the modification time: an edit within the
    /// filesystem's timestamp granularity (or one that restores the time) must still take a revoked device out at once.
    trust_file: std::sync::Mutex<Option<(std::path::PathBuf, Option<Vec<u8>>)>>,
}

impl NodeApprovals {
    pub fn new(node_id: &str, verifier: Box<dyn Verifier + Send>) -> Self {
        Self {
            queue: std::sync::Mutex::new(Approvals::new(node_id, verifier)),
            routes: std::sync::Mutex::new(std::collections::HashMap::new()),
            trust_file: std::sync::Mutex::new(None),
        }
    }

    /// Follows the trusted-devices file: when the owner changes it (`agora node trust`), the next decision is judged by
    /// the new list, so trusting a browser does not mean restarting the node (which would stop its agents).
    pub fn follow_trust_file(&self, path: &std::path::Path) {
        let seen = std::fs::read(path).ok();
        *self.trust_file.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((path.to_path_buf(), seen));
    }

    fn reload_trust_if_changed(&self) {
        let mut guard = self.trust_file.lock().unwrap_or_else(|e| e.into_inner());
        let Some((path, seen)) = guard.as_mut() else {
            return;
        };
        let now = std::fs::read(&*path).ok();
        if now != *seen {
            *seen = now;
            let (verifier, _skipped) = P256Verifier::load(path);
            self.set_verifier(Box::new(verifier));
        }
    }

    /// Replaces the verifier; requests already waiting are judged by the new one.
    pub fn set_verifier(&self, verifier: Box<dyn Verifier + Send>) {
        self.q().set_verifier(verifier);
    }

    fn q(&self) -> std::sync::MutexGuard<'_, Approvals<Box<dyn Verifier + Send>>> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn r(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, (agora_core::AgentId, String)>>
    {
        self.routes.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// An adapter asked. Returns the pending request to announce, or `None` if no safe id could be made.
    pub fn request(
        &self,
        agent: &agora_core::AgentId,
        adapter_approval_id: &str,
        action: &str,
        detail: &str,
        now_ms: u64,
    ) -> Option<Pending> {
        let p = self
            .q()
            .request(agent.as_str(), action, detail, now_ms, 0)?;
        self.r().insert(
            p.id.clone(),
            (agent.clone(), adapter_approval_id.to_string()),
        );
        Some(p)
    }

    /// A signed decision arrived. On success, the adapter's own approval id and agent to answer, and the verdict.
    pub fn resolve(
        &self,
        r: &ApprovalResolve,
        now_ms: u64,
    ) -> Result<(agora_core::AgentId, String, Verdict), ApprovalError> {
        self.reload_trust_if_changed();
        let (_, verdict) = self.q().resolve(r, now_ms)?;
        let route = self.r().remove(&r.approval_id);
        route
            .map(|(agent, id)| (agent, id, verdict))
            .ok_or(ApprovalError::Unknown)
    }

    /// Requests that ran out of time: the agent behind each is told "denied". Returns `(node approval id, agent,
    /// adapter approval id)`.
    pub fn expire(&self, now_ms: u64) -> Vec<(String, agora_core::AgentId, String)> {
        let gone = self.q().expire(now_ms);
        let mut routes = self.r();
        gone.into_iter()
            .filter_map(|p| routes.remove(&p.id).map(|(a, id)| (p.id, a, id)))
            .collect()
    }

    /// An agent stopped or died: nothing stays waiting on it.
    pub fn cancel_agent(&self, agent: &agora_core::AgentId) {
        let gone = self.q().cancel_agent(agent.as_str());
        let mut routes = self.r();
        for p in gone {
            routes.remove(&p.id);
        }
    }

    pub fn pending_count(&self) -> usize {
        self.q().pending().count()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// A stand-in scheme for tests: a device "signs" by prefixing its secret to the message. The real scheme
    /// differs; what matters here is that verification is tied to the device and the exact message.
    struct Fake(Vec<(&'static str, &'static str)>);

    impl Verifier for Fake {
        fn verify(&self, device: &str, message: &[u8], signature: &[u8]) -> bool {
            self.0
                .iter()
                .any(|(d, secret)| *d == device && sign(secret, message) == signature)
        }
    }

    fn sign(secret: &str, message: &[u8]) -> Vec<u8> {
        let mut s = secret.as_bytes().to_vec();
        s.extend_from_slice(message);
        s
    }

    fn approvals() -> Approvals<Fake> {
        Approvals::new(
            "node-1",
            Fake(vec![("phone", "s3cret"), ("laptop", "other")]),
        )
    }

    fn resolve_msg(
        id: &str,
        approved: bool,
        device: &str,
        secret: &str,
        node: &str,
    ) -> ApprovalResolve {
        ApprovalResolve {
            approval_id: id.into(),
            approved,
            signer_device_id: device.into(),
            signature: sign(secret, &signed_message(node, id, approved)),
        }
    }

    #[test]
    fn a_signed_decision_applies_once() {
        let mut a = approvals();
        let p = a
            .request("agent-1", "run_command", "rm -rf build", 1000, 0)
            .unwrap();
        assert!(p.id.starts_with("ap-") && p.id.len() == 35);
        assert_eq!(a.pending().count(), 1);
        let r = resolve_msg(&p.id, true, "phone", "s3cret", "node-1");
        let (got, v) = a.resolve(&r, 2000).unwrap();
        assert_eq!((got.agent_id.as_str(), v), ("agent-1", Verdict::Approved));
        assert_eq!(a.pending().count(), 0);
        // The same signed message again is a replay: refused.
        assert_eq!(a.resolve(&r, 2001), Err(ApprovalError::Unknown));
        assert!(a.was_spent(&p.id));
    }

    #[test]
    fn a_denial_is_a_verdict_too() {
        let mut a = approvals();
        let p = a.request("agent-1", "write_file", "x", 0, 0).unwrap();
        let (_, v) = a
            .resolve(&resolve_msg(&p.id, false, "laptop", "other", "node-1"), 1)
            .unwrap();
        assert_eq!(v, Verdict::Denied);
    }

    #[test]
    fn unknown_devices_and_bad_signatures_change_nothing() {
        let mut a = approvals();
        let p = a.request("agent-1", "run_command", "x", 0, 0).unwrap();
        for r in [
            resolve_msg(&p.id, true, "stranger", "s3cret", "node-1"),
            resolve_msg(&p.id, true, "phone", "wrong", "node-1"),
        ] {
            assert_eq!(a.resolve(&r, 1), Err(ApprovalError::BadSignature));
        }
        assert_eq!(a.pending().count(), 1, "still waiting for a real decision");
    }

    #[test]
    fn a_signature_cannot_be_moved_to_another_node_flipped_or_reused_for_another_request() {
        let mut a = approvals();
        let p1 = a.request("agent-1", "run_command", "x", 0, 0).unwrap();
        let p2 = a.request("agent-1", "run_command", "y", 0, 0).unwrap();
        // Signed for another node.
        let other_node = resolve_msg(&p1.id, true, "phone", "s3cret", "node-2");
        assert_eq!(a.resolve(&other_node, 1), Err(ApprovalError::BadSignature));
        // A "deny" signature presented as an "allow".
        let mut flipped = resolve_msg(&p1.id, false, "phone", "s3cret", "node-1");
        flipped.approved = true;
        assert_eq!(a.resolve(&flipped, 1), Err(ApprovalError::BadSignature));
        // A signature for request 1 used on request 2.
        let mut moved = resolve_msg(&p1.id, true, "phone", "s3cret", "node-1");
        moved.approval_id = p2.id.clone();
        assert_eq!(a.resolve(&moved, 1), Err(ApprovalError::BadSignature));
        assert_eq!(a.pending().count(), 2);
    }

    #[test]
    fn late_decisions_are_refused_and_expiry_sweeps() {
        let mut a = approvals();
        let p = a
            .request("agent-1", "run_command", "x", 1000, 5000)
            .unwrap();
        let late = resolve_msg(&p.id, true, "phone", "s3cret", "node-1");
        assert_eq!(a.resolve(&late, 6000), Err(ApprovalError::Expired));
        assert_eq!(a.pending().count(), 0);
        let q = a.request("agent-2", "run_command", "x", 0, 100).unwrap();
        a.request("agent-3", "run_command", "x", 0, 10_000).unwrap();
        let gone = a.expire(100);
        assert_eq!(
            gone.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
            [q.id]
        );
        assert_eq!(a.pending().count(), 1);
    }

    #[test]
    fn an_unknown_id_looks_the_same_whether_never_issued_or_spent() {
        let mut a = approvals();
        let r = resolve_msg("ap-nope", true, "phone", "s3cret", "node-1");
        assert_eq!(a.resolve(&r, 0), Err(ApprovalError::Unknown));
    }

    #[test]
    fn a_stopped_agent_leaves_nothing_waiting() {
        let mut a = approvals();
        a.request("agent-1", "a", "", 0, 0).unwrap();
        a.request("agent-1", "b", "", 0, 0).unwrap();
        a.request("agent-2", "c", "", 0, 0).unwrap();
        assert_eq!(a.cancel_agent("agent-1").len(), 2);
        assert_eq!(
            a.pending().map(|p| p.agent_id.as_str()).collect::<Vec<_>>(),
            ["agent-2"]
        );
    }

    #[test]
    fn ids_are_not_repeated() {
        let mut a = approvals();
        let ids: std::collections::BTreeSet<String> = (0..100)
            .map(|_| a.request("a", "x", "", 0, 0).unwrap().id)
            .collect();
        assert_eq!(ids.len(), 100);
    }

    #[test]
    fn policy_matches_exact_names_prefixes_and_everything() {
        let p = Policy {
            approval_required: vec!["run_command".into(), "install:*".into()],
        };
        assert!(p.requires("run_command"));
        assert!(p.requires("install:adapter"));
        assert!(!p.requires("run_commands"));
        assert!(!p.requires("write_file"));
        assert!(Policy {
            approval_required: vec!["*".into()]
        }
        .requires("anything"));
        assert!(!Policy::default().requires("run_command"));
    }

    #[test]
    fn the_signed_message_is_domain_separated_and_unambiguous() {
        let m = signed_message("n", "a", true);
        assert!(m.starts_with(b"agora-approval-v1\0"));
        assert_ne!(
            signed_message("n", "a", true),
            signed_message("n", "a", false)
        );
        // "ab" + "c" must not collide with "a" + "bc".
        assert_ne!(
            signed_message("ab", "c", true),
            signed_message("a", "bc", true)
        );
    }

    #[test]
    fn the_trusted_devices_file_is_read_strictly() {
        use ring::rand::SystemRandom;
        use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
            .unwrap();
        let hex: String = pair
            .public_key()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let dir = std::env::temp_dir().join(format!("agora-trust-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(TRUSTED_DEVICES_FILE);
        assert_eq!(
            P256Verifier::load(&file).1,
            0,
            "a missing file trusts nothing"
        );

        trust_device(&file, "phone", &hex).unwrap();
        assert!(trust_device(&file, "bad id", &hex).is_err());
        assert!(trust_device(&file, "x", "zz").is_err());
        assert!(trust_device(&file, "x", "0401").is_err());
        trust_device(&file, "phone", &hex).unwrap(); // replaces, no duplicate line
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 1);

        let mut text = std::fs::read_to_string(&file).unwrap();
        text.push_str("# a comment\n\nlaptop 04deadbeef\njustoneword\n");
        std::fs::write(&file, text).unwrap();
        let (v, skipped) = P256Verifier::load(&file);
        assert_eq!(skipped, 2, "malformed lines are skipped, never trusted");
        let msg = signed_message("node-1", "ap-1", true);
        let sig = pair.sign(&rng, &msg).unwrap();
        assert!(v.verify("phone", &msg, sig.as_ref()));
        assert!(!v.verify("laptop", &msg, sig.as_ref()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_device_trusted_while_the_node_runs_can_answer_without_a_restart() {
        use ring::rand::SystemRandom;
        use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
            .unwrap();
        let hex: String = pair
            .public_key()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let dir = std::env::temp_dir().join(format!("agora-trust-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(TRUSTED_DEVICES_FILE);

        let n = NodeApprovals::new("node-1", Box::new(P256Verifier::load(&file).0));
        n.follow_trust_file(&file);
        let agent = agora_core::AgentId::new("agent-1");
        let p = n.request(&agent, "ap-1", "run_command", "ls", 0).unwrap();
        let sign = |approved: bool| {
            let sig = pair
                .sign(&rng, &signed_message("node-1", &p.id, approved))
                .unwrap();
            ApprovalResolve {
                approval_id: p.id.clone(),
                approved,
                signer_device_id: "web-1".into(),
                signature: sig.as_ref().to_vec(),
            }
        };
        assert_eq!(
            n.resolve(&sign(true), 1).unwrap_err(),
            ApprovalError::BadSignature,
            "not trusted yet"
        );

        // The owner trusts the device on the machine; the same node, with the same waiting request, now accepts it.
        std::thread::sleep(std::time::Duration::from_millis(20));
        trust_device(&file, "web-1", &hex).unwrap();
        let (a, id, verdict) = n.resolve(&sign(true), 2).unwrap();
        assert_eq!(
            (a.as_str(), id.as_str(), verdict),
            ("agent-1", "ap-1", Verdict::Approved)
        );

        // The owner removes the device again, and the edit keeps the file's modification time (a coarse clock, or a copy that
        // preserves it): the device must still be out on the very next decision.
        let before = std::fs::metadata(&file).unwrap().modified().unwrap();
        std::fs::write(&file, "").unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(before)
            .unwrap();
        let p2 = n.request(&agent, "ap-2", "run_command", "ls", 3).unwrap();
        let sig = pair
            .sign(&rng, &signed_message("node-1", &p2.id, true))
            .unwrap();
        let resolve = ApprovalResolve {
            approval_id: p2.id.clone(),
            approved: true,
            signer_device_id: "web-1".into(),
            signature: sig.as_ref().to_vec(),
        };
        assert_eq!(
            n.resolve(&resolve, 4).unwrap_err(),
            ApprovalError::BadSignature
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn p256_signatures_verify_only_for_the_trusted_device_and_exact_message() {
        use ring::rand::SystemRandom;
        use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
            .unwrap();
        let mut v = P256Verifier::new();
        assert!(v.trust("phone", pair.public_key().as_ref()));
        assert!(!v.trust("bad", &[1, 2, 3]));
        let msg = signed_message("node-1", "ap-1", true);
        let sig = pair.sign(&rng, &msg).unwrap();
        assert!(v.verify("phone", &msg, sig.as_ref()));
        assert!(!v.verify("laptop", &msg, sig.as_ref()), "unknown device");
        assert!(
            !v.verify(
                "phone",
                &signed_message("node-1", "ap-1", false),
                sig.as_ref()
            ),
            "other verdict"
        );
        assert!(!v.verify("phone", &msg, &sig.as_ref()[..63]), "truncated");
        v.revoke("phone");
        assert!(!v.verify("phone", &msg, sig.as_ref()));
        // And through the whole queue.
        let mut v = P256Verifier::new();
        v.trust("phone", pair.public_key().as_ref());
        let mut a = Approvals::new("node-1", v);
        let p = a.request("agent-1", "run_command", "ls", 0, 0).unwrap();
        let m = signed_message("node-1", &p.id, true);
        let r = ApprovalResolve {
            approval_id: p.id.clone(),
            approved: true,
            signer_device_id: "phone".into(),
            signature: pair.sign(&rng, &m).unwrap().as_ref().to_vec(),
        };
        assert_eq!(a.resolve(&r, 1).unwrap().1, Verdict::Approved);
    }
}
