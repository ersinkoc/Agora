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
}
