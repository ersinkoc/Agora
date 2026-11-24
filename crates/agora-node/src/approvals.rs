//! Approvals on the node (ADR-0020, threat T1): sensitive actions wait here until a user's device signs a decision
//! and the node verifies that signature itself. The hub only relays; a compromised hub can neither approve nor
//! widen the policy. Mesh content and agent output never reach this decision (T4).
//!
//! What is enforced:
//! - the signature must verify under a key the node already trusts for that device id (unknown signer: refused);
//! - the signed message binds this node, this approval and the verdict, so a decision cannot be replayed on
//!   another node, for another approval, or flipped from deny to allow;
//! - approval ids are random and single use: a decision applies once, then the id is spent (replay refused). ADR-0110 decision 6:
//!   a request whose answer the adapter did not take in time is put back so it can be answered again; the signatures of the
//!   answers already taken for it are kept and an identical one is refused, so a hub cannot replay the person's earlier answer over
//!   a newer one (every signer signs with a random nonce: WebCrypto ECDSA, and `ring` with `SystemRandom` in the terminal client);
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

    /// The term `device_id` is trusted for, asked only after [`Self::verify`] accepted its signature: what lets a node tell later
    /// that a verdict it gave still stands (a grant, audit F270). A verifier that keeps no record of when trust began or ends (the
    /// test verifiers) has one term without end.
    fn term(&self, _device_id: &str) -> Option<Term> {
        Some(Term::default())
    }
}

/// One stretch of a device's trust on this node (audit F270): when it began (`since` in the trusted-devices file, `None` for a line
/// without one) and when it ends by itself (an endorsed device, ADR-0061; `None`: no end). Trusting a device again starts a new term.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Term {
    pub since: Option<u64>,
    pub until: Option<u64>,
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

/// The action of an agent's request to start another agent (ADR-0057).
pub const START_ACTION: &str = "agent.start";

/// What a device signs to decide an `agent.start` request (ADR-0057): the decision, the grant that comes with it, and a hash of the
/// request exactly as it was announced, so the decision binds what the user was shown (a hub cannot swap the prompt, the folder or
/// the kind, nor add or strip a grant).
pub fn start_signed_message(
    node_id: &str,
    approval_id: &str,
    approved: bool,
    helper_grant: u32,
    helpers_may_start: bool,
    detail: &str,
) -> Vec<u8> {
    let digest = ring::digest::digest(&ring::digest::SHA256, detail.as_bytes());
    let hash: String = digest.as_ref().iter().map(|b| format!("{b:02x}")).collect();
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

/// ADR-0066: the launch settings an `agent.start` request's detail announces (`settings`: a list of `{id, value}`, in the order they
/// were shown), as pairs. Empty for a detail without any, and for one that does not parse (the node wrote the detail itself, so that
/// is not a case that happens; a start request without settings is signed as it always was).
pub fn detail_settings(detail: &str) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(detail) else {
        return Vec::new();
    };
    let Some(list) = v.get("settings").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .map(|s| {
            let get = |k: &str| {
                s.get(k)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            (get("id"), get("value"))
        })
        .collect()
}

/// ADR-0066: what a device signs to decide an `agent.start` request that carries launch settings: the fields of
/// [`start_signed_message`] under the domain `agora-approval-start-v2`, with the settings as a length-prefixed pair list
/// ([`crate::directory::signed_pairs`]) before the hash of the detail. A request with settings verifies only over this message (a
/// client that signs the v1 message, which does not name the settings, is refused), and one without keeps v1. Shared vectors:
/// `schema/fixtures/signing/approval-start-v2.json`.
pub fn start_signed_message_v2(
    node_id: &str,
    approval_id: &str,
    approved: bool,
    helper_grant: u32,
    helpers_may_start: bool,
    settings: &[(String, String)],
    detail: &str,
) -> Vec<u8> {
    let digest = ring::digest::digest(&ring::digest::SHA256, detail.as_bytes());
    let hash: String = digest.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    let mut m = b"agora-approval-start-v2".to_vec();
    for part in [
        node_id,
        approval_id,
        if approved { "1" } else { "0" },
        &helper_grant.to_string(),
        if helpers_may_start { "1" } else { "0" },
        &crate::directory::signed_pairs(settings),
        &hash,
    ] {
        m.push(0);
        m.extend_from_slice(part.as_bytes());
    }
    m
}

/// ADR-0059 decision 5: the launch option ids an `agent.start` request's detail announces (`launch_options`: a list of `{id, label,
/// skips_permissions}`, in the order they were shown). Empty for a detail without any, and for one that does not parse (the node wrote
/// the detail itself; a start request without options is signed as it always was).
pub fn detail_launch_options(detail: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(detail) else {
        return Vec::new();
    };
    let Some(list) = v
        .get("launch_options")
        .and_then(serde_json::Value::as_array)
    else {
        return Vec::new();
    };
    list.iter()
        .map(|o| {
            o.get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string()
        })
        .collect()
}

/// ADR-0059 decision 5: what a device signs to decide an `agent.start` request that carries launch options (a helper that acts without
/// asking): the fields of [`start_signed_message_v2`] under the domain `agora-approval-start-v3`, then the option ids as a
/// length-prefixed list ([`crate::directory::signed_list`]) before the hash of the detail. A request with options verifies only over
/// this message (a v1 or v2 signature, which does not name them, is refused); one without keeps v1 or v2. Shared vectors:
/// `schema/fixtures/signing/approval-start-v3.json`.
#[allow(clippy::too_many_arguments)]
pub fn start_signed_message_v3(
    node_id: &str,
    approval_id: &str,
    approved: bool,
    helper_grant: u32,
    helpers_may_start: bool,
    settings: &[(String, String)],
    options: &[String],
    detail: &str,
) -> Vec<u8> {
    let digest = ring::digest::digest(&ring::digest::SHA256, detail.as_bytes());
    let hash: String = digest.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    let mut m = b"agora-approval-start-v3".to_vec();
    for part in [
        node_id,
        approval_id,
        if approved { "1" } else { "0" },
        &helper_grant.to_string(),
        if helpers_may_start { "1" } else { "0" },
        &crate::directory::signed_pairs(settings),
        &crate::directory::signed_list(options),
        &hash,
    ] {
        m.push(0);
        m.extend_from_slice(part.as_bytes());
    }
    m
}

/// ADR-0090 Amendment A: the action of a request to switch a running agent to a mode or option that skips permission questions. Only
/// the node raises it ([`NodeApprovals::request_skip`]); a request an adapter makes under this name is refused.
pub const SKIP_ACTION: &str = "agent.skip_permissions";

/// Actions only the node raises: the whole `agent.` namespace (an adapter cannot dress its own request as one of them).
pub fn is_reserved_action(action: &str) -> bool {
    action == START_ACTION || action == SKIP_ACTION || action.starts_with("agent.")
}

/// `F(x)`: the decimal byte length of `x`, `:`, then `x` (the field form of the newer signing domains).
fn framed(out: &mut Vec<u8>, x: &str) {
    out.extend_from_slice(x.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(x.as_bytes());
}

/// ADR-0090 Amendment A: what a device signs to decide an `agent.skip_permissions` request: the domain `agora-approval-skip-v1`, one
/// NUL, then `F(node_id) F(approval_id) F(verdict) F(agent_id) F(option_id) F(value) F(sha256(detail))` (verdict `1` or `0`, the hash
/// of the detail exactly as announced in lowercase hex). The decision binds the agent, the option and the value as well as the
/// words the user read. Shared vectors: `schema/fixtures/signing/approval-skip-v1.json`.
pub fn skip_signed_message(
    node_id: &str,
    approval_id: &str,
    approved: bool,
    binding: &SkipBinding,
    detail: &str,
) -> Vec<u8> {
    let digest = ring::digest::digest(&ring::digest::SHA256, detail.as_bytes());
    let hash: String = digest.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    let mut m = b"agora-approval-skip-v1\0".to_vec();
    for part in [
        node_id,
        approval_id,
        if approved { "1" } else { "0" },
        &binding.agent,
        &binding.option,
        &binding.value,
        &hash,
    ] {
        framed(&mut m, part);
    }
    m
}

/// ADR-0101: what a device signs to allow an adapter's request always: the domain `agora-approval-always-v1`, one NUL, then
/// `F(node_id) F(approval_id) F(verdict) F(agent_id) F(sha256(action)) F(sha256(detail)) F(always_label) F(always_option_id)`, the verdict
/// always `1` (an always decision is never a denial), the hashes in lowercase hex over the action and detail exactly as announced, and
/// the label and option id exactly as announced. Shared vectors: `schema/fixtures/signing/approval-always-v1.json`.
pub fn always_signed_message(node_id: &str, approval_id: &str, request: &Pending) -> Vec<u8> {
    let hash = |s: &str| -> String {
        ring::digest::digest(&ring::digest::SHA256, s.as_bytes())
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    };
    let (label, option) = request
        .offer
        .always
        .as_ref()
        .map_or(("", ""), |a| (a.label.as_str(), a.option_id.as_str()));
    let mut m = b"agora-approval-always-v1\0".to_vec();
    for part in [
        node_id,
        approval_id,
        "1",
        &request.agent_id,
        &hash(&request.action),
        &hash(&request.detail),
        label,
        option,
    ] {
        framed(&mut m, part);
    }
    m
}

/// ADR-0101: the longest agent label for an always option a card shows (characters).
pub const MAX_ALWAYS_LABEL_CHARS: usize = 120;

/// ADR-0101: the agent's always option as announced: its own name (cleaned, bounded) and its exact id (signable as it is).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlwaysOption {
    pub label: String,
    pub option_id: String,
}

/// ADR-0101: the answers a request offers the person, after the node's policy: what the node announced and what a decision is checked
/// against. A request made before ADR-0101, and every node-made one, offers the single answer only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub once: bool,
    pub always: Option<AlwaysOption>,
}

impl Default for Offer {
    fn default() -> Self {
        Self::once_only()
    }
}

impl Offer {
    pub fn once_only() -> Self {
        Self {
            once: true,
            always: None,
        }
    }

    /// `ApprovalRequested.scopes` (empty for the single answer only, as every node before ADR-0101 said it).
    pub fn scopes(&self) -> Vec<i32> {
        if self.always.is_none() {
            return Vec::new();
        }
        let mut out = Vec::new();
        if self.once {
            out.push(agora_protocol::v1::ApprovalScope::Once as i32);
        }
        out.push(agora_protocol::v1::ApprovalScope::Always as i32);
        out
    }
}

/// ADR-0101: what the node makes of an adapter's request, given its offer and the policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Raise {
    /// A card offering these answers.
    Card(Offer),
    /// The agent offered only an answer it would remember, and the policy (or the kind's manifest) refuses that: no card, the
    /// adapter is answered denied at once, and the person is told (`approval.always_only_forbidden`).
    AlwaysOnlyForbidden,
    /// Nothing the person could allow (an always option whose id cannot be signed as it is, and no single answer): no card, denied,
    /// and the person is told (`adapter.permission_unanswerable`).
    Unanswerable,
}

/// ADR-0101: the card for an adapter's request. `strict`: the adapter declared `approval_scopes` (its `scopes` are read, and it never
/// answers a single Allow with a remembered option); otherwise the request offers the single answer only, whatever it says.
/// `always_permitted`: the node's `skip_permissions` policy and the kind's manifest let this agent be allowed always.
pub fn raise_for(
    strict: bool,
    scopes: &[agora_protocol::v1::ApprovalScope],
    always_label: &str,
    always_option_id: &str,
    always_permitted: bool,
) -> Raise {
    use agora_protocol::v1::ApprovalScope;
    if !strict || scopes.is_empty() {
        return Raise::Card(Offer::once_only());
    }
    let once = scopes.contains(&ApprovalScope::Once);
    let offered = scopes.contains(&ApprovalScope::Always);
    let always = (offered && signable(always_option_id)).then(|| AlwaysOption {
        label: card_text(always_label, MAX_ALWAYS_LABEL_CHARS),
        option_id: always_option_id.to_string(),
    });
    match (once, always) {
        (once, Some(a)) if always_permitted => Raise::Card(Offer {
            once,
            always: Some(a),
        }),
        (true, _) => Raise::Card(Offer::once_only()),
        (false, Some(_)) => Raise::AlwaysOnlyForbidden,
        (false, None) => Raise::Unanswerable,
    }
}

/// ADR-0101: what the node's policy says when a decision arrives: the `skip_permissions` line the node read at start, for the agent's
/// launch as it is then (a card raised by an earlier node, or for a process replaced since, is checked against the current ones).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decide {
    /// The policy and the kind's manifest let this agent be allowed always now.
    pub always: bool,
    /// A single Allow may be delivered. False only for an agent behind an ACP bridge from before ADR-0101 (it did not declare
    /// `approval_scopes`, so it may answer a single Allow with a remembered option) under a policy that refuses "always".
    pub once: bool,
}

impl Decide {
    /// The policy for an adapter agent's request: `always_permitted` (the `skip_permissions` policy and the manifest), `strict` (its
    /// adapter declared `approval_scopes`) and `acp_bridge` (it runs behind the ACP bridge). "Always" only through a strict adapter; a
    /// single Allow refused only through a bridge that is not strict while "always" is not permitted (Codex review of ADR-0101: such a
    /// bridge, from before ADR-0101, may answer it with the agent's remembered option).
    pub fn for_adapter(always_permitted: bool, strict: bool, acp_bridge: bool) -> Self {
        Self {
            always: always_permitted && strict,
            once: always_permitted || strict || !acp_bridge,
        }
    }
}

impl Default for Decide {
    /// What a caller that does not know the policy gets: never "always" (fail closed), single answers as before.
    fn default() -> Self {
        Self {
            always: false,
            once: true,
        }
    }
}

/// The agent, option and value a skip decision is bound to (the signed `agent_id`, `option_id` and `value`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkipBinding {
    pub agent: String,
    pub option: String,
    pub value: String,
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
    /// Only for [`SKIP_ACTION`]: what the decision is bound to, taken from the node's own record of the request, never from the detail.
    pub binding: Option<SkipBinding>,
    /// ADR-0101: the answers announced (after the policy); a decision is checked against this, never against anything it carries.
    pub offer: Offer,
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
    /// The device is limited by the node's policy to other projects than this request's (ADR-0049).
    Forbidden,
    /// ADR-0101: a scope this build does not know.
    InvalidScope,
    /// ADR-0101: the request did not offer that answer (`approval.scope_unavailable`). Nothing was spent.
    ScopeUnavailable,
    /// ADR-0101: the policy refuses "always" for this agent now (`approval.scope_forbidden`). Nothing was spent.
    ScopeForbidden,
    /// ADR-0101 (Codex review): a single Allow through an ACP bridge from before ADR-0101 under a policy that refuses "always"
    /// (`approval.adapter_outdated`). Nothing was spent.
    AdapterOutdated,
}

pub struct Approvals<V: Verifier> {
    node_id: String,
    verifier: V,
    /// How many times the verifier was replaced: a verdict given under an older one must be checked again (audit F270).
    generation: u64,
    pending: BTreeMap<String, Pending>,
    spent: VecDeque<String>,
    /// ADR-0110 (security review of PR #1190, S1): each answer taken, by request id, in [`replay_form`], so an answer to a request
    /// that was put back ([`Self::restore`]) cannot be the same signed answer again. Bounded like `spent` (it shares that cap).
    answered: VecDeque<(String, Vec<u8>)>,
}

/// Re-review of PR #1190, R1: what an answer's signature is compared by to refuse a replay. An ECDSA P-256 signature (`r || s`, 64
/// bytes, the only form [`P256Verifier`] accepts) also verifies with `s` replaced by `n - s`, so anyone relaying it can change those
/// bytes without the key; `r` cannot change without signing again. A signer signs with a fresh random nonce each time (WebCrypto
/// ECDSA in the browser, `ring` with `SystemRandom` in the terminal client), so a genuine new answer has another `r`. Any other
/// length (a test verifier's scheme) is compared whole.
fn replay_form(signature: &[u8]) -> Vec<u8> {
    match signature.len() {
        64 => signature[..32].to_vec(),
        _ => signature.to_vec(),
    }
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
            generation: 0,
            pending: BTreeMap::new(),
            spent: VecDeque::new(),
            answered: VecDeque::new(),
        }
    }

    /// Replaces the verifier (when trusted device keys are loaded or changed).
    pub fn set_verifier(&mut self, verifier: V) {
        self.verifier = verifier;
        self.generation = self.generation.wrapping_add(1);
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
        self.request_bound(agent_id, action, detail, now_ms, ttl_ms, None)
    }

    /// ADR-0101: [`Self::request`] offering the answers in `offer`.
    pub fn request_offered(
        &mut self,
        agent_id: &str,
        action: &str,
        detail: &str,
        now_ms: u64,
        offer: Offer,
    ) -> Option<Pending> {
        let mut p = self.request_bound(agent_id, action, detail, now_ms, 0, None)?;
        p.offer = offer;
        if let Some(stored) = self.pending.get_mut(&p.id) {
            stored.offer = p.offer.clone();
        }
        Some(p)
    }

    /// [`Self::request`] with the binding a skip decision is signed over.
    pub fn request_bound(
        &mut self,
        agent_id: &str,
        action: &str,
        detail: &str,
        now_ms: u64,
        ttl_ms: u64,
        binding: Option<SkipBinding>,
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
            binding,
            offer: Offer::once_only(),
        };
        self.pending.insert(id, p.clone());
        Some(p)
    }

    pub fn pending(&self) -> impl Iterator<Item = &Pending> {
        self.pending.values()
    }

    /// Applies a signed decision. On success the request is removed and its id spent. Never "always" (see [`Self::resolve_decided`]).
    pub fn resolve(
        &mut self,
        r: &ApprovalResolve,
        now_ms: u64,
    ) -> Result<(Pending, Verdict), ApprovalError> {
        self.resolve_decided(r, now_ms, Decide::default())
    }

    /// Applies a signed decision under the policy `decide` (ADR-0101). The checks, in order: the scope's shape (a value this build does
    /// not know; ALWAYS with a denial or on a node-made request), which reveals nothing about the request; the signature, over the bytes
    /// the decision's scope names (ALWAYS: `agora-approval-always-v1`, anything else: as before), so a hub that changes the scope in
    /// either direction fails here; then the offer stored with the request, the policy, and expiry. A refusal before expiry spends
    /// nothing: the card stays waiting.
    pub fn resolve_decided(
        &mut self,
        r: &ApprovalResolve,
        now_ms: u64,
        decide: Decide,
    ) -> Result<(Pending, Verdict), ApprovalError> {
        use agora_protocol::v1::ApprovalScope;
        let always = match ApprovalScope::try_from(r.scope) {
            Ok(ApprovalScope::Unspecified | ApprovalScope::Once) => false,
            Ok(ApprovalScope::Always) => true,
            Err(_) => return Err(ApprovalError::InvalidScope),
        };
        let Some(p) = self.pending.get(&r.approval_id) else {
            return Err(ApprovalError::Unknown);
        };
        // S1: a request put back after its answer did not reach the adapter takes a new answer, never one already given (compared
        // in a form a relay cannot change without the key: [`replay_form`]).
        let form = replay_form(&r.signature);
        if self
            .answered
            .iter()
            .any(|(id, sig)| *id == r.approval_id && *sig == form)
        {
            return Err(ApprovalError::BadSignature);
        }
        if always {
            // An always answer is an Allow on an adapter's request, never a denial ("deny always" is reserved) and never a node-made
            // request (those have signed grants of their own); a grant means nothing here.
            if !r.approved
                || is_reserved_action(&p.action)
                || r.helper_grant != 0
                || r.helpers_may_start
            {
                return Err(ApprovalError::BadSignature);
            }
            let msg = always_signed_message(&self.node_id, &r.approval_id, p);
            if !self
                .verifier
                .verify(&r.signer_device_id, &msg, &r.signature)
            {
                return Err(ApprovalError::BadSignature);
            }
            if p.offer.always.is_none() {
                return Err(ApprovalError::ScopeUnavailable);
            }
            if !decide.always {
                return Err(ApprovalError::ScopeForbidden);
            }
            return self.take(r, now_ms);
        }
        // Verify before anything changes, and before saying whether the request expired: a peer without a trusted
        // key cannot learn the state of a request it can name (ids are 128 random bits, so it cannot guess one).
        let (settings, options) = if p.action == START_ACTION {
            (detail_settings(&p.detail), detail_launch_options(&p.detail))
        } else {
            (Vec::new(), Vec::new())
        };
        let msg = if p.action == SKIP_ACTION {
            // Only the skip domain decides it, and it carries no grant: anything else on it is refused. A request without a binding
            // (it cannot exist: the node makes one only through `request_bound`) is never decided.
            let Some(binding) = &p.binding else {
                return Err(ApprovalError::BadSignature);
            };
            if r.helper_grant != 0 || r.helpers_may_start {
                return Err(ApprovalError::BadSignature);
            }
            skip_signed_message(
                &self.node_id,
                &r.approval_id,
                r.approved,
                binding,
                &p.detail,
            )
        } else if p.action == START_ACTION && !options.is_empty() {
            start_signed_message_v3(
                &self.node_id,
                &r.approval_id,
                r.approved,
                r.helper_grant,
                r.helpers_may_start,
                &settings,
                &options,
                &p.detail,
            )
        } else if p.action == START_ACTION && !settings.is_empty() {
            start_signed_message_v2(
                &self.node_id,
                &r.approval_id,
                r.approved,
                r.helper_grant,
                r.helpers_may_start,
                &settings,
                &p.detail,
            )
        } else if p.action == START_ACTION {
            start_signed_message(
                &self.node_id,
                &r.approval_id,
                r.approved,
                r.helper_grant,
                r.helpers_may_start,
                &p.detail,
            )
        } else if r.helper_grant != 0 || r.helpers_may_start {
            // A grant means something only for a start; on any other request nothing would bind it, so it is refused.
            return Err(ApprovalError::BadSignature);
        } else {
            signed_message(&self.node_id, &r.approval_id, r.approved)
        };
        if !self
            .verifier
            .verify(&r.signer_device_id, &msg, &r.signature)
        {
            return Err(ApprovalError::BadSignature);
        }
        // ADR-0101: a single Allow on a request that offers only "always" would be remembered by the agent: refused, the card stays.
        // A denial needs no offer (it never widens anything).
        if r.approved && !p.offer.once {
            return Err(ApprovalError::ScopeUnavailable);
        }
        if r.approved && !decide.once && !is_reserved_action(&p.action) {
            return Err(ApprovalError::AdapterOutdated);
        }
        self.take(r, now_ms)
    }

    /// The decision verified and passed every check: expiry, then the request is removed and its id spent.
    fn take(
        &mut self,
        r: &ApprovalResolve,
        now_ms: u64,
    ) -> Result<(Pending, Verdict), ApprovalError> {
        let Some(p) = self.pending.get(&r.approval_id) else {
            return Err(ApprovalError::Unknown);
        };
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
        self.answered
            .push_back((r.approval_id.clone(), replay_form(&r.signature)));
        // The same bound as the spent ids: an id whose answer fell out of it is long past any request that can still be put back.
        while self.answered.len() > SPENT_MEMORY {
            self.answered.pop_front();
        }
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

    /// ADR-0110 decision 6: puts back a request a verified decision took out ([`Self::resolve_decided`]) whose answer did not reach
    /// the adapter in time, so it can be answered again. Its id is no longer spent.
    pub fn restore(&mut self, p: Pending) {
        self.spent.retain(|s| *s != p.id);
        self.pending.insert(p.id.clone(), p);
    }

    /// Withdraws one request (its asker no longer waits for it); the id is spent, so a late decision is refused.
    pub fn withdraw(&mut self, id: &str) -> Option<Pending> {
        let p = self.pending.remove(id)?;
        self.spend(id);
        Some(p)
    }

    fn spend(&mut self, id: &str) {
        self.spent.push_back(id.to_string());
        while self.spent.len() > SPENT_MEMORY {
            self.spent.pop_front();
        }
    }

    /// The verifier's answer, for callers that sign other things with the same devices (grants, ADR-0041).
    pub fn verify_with(&self, device_id: &str, message: &[u8], signature: &[u8]) -> bool {
        self.verifier.verify(device_id, message, signature)
    }

    /// Like [`Self::verify_with`], with the verifier's generation and the device's term when the signature verifies.
    pub fn verify_term(
        &self,
        device_id: &str,
        message: &[u8],
        signature: &[u8],
    ) -> Option<(u64, Term)> {
        if !self.verifier.verify(device_id, message, signature) {
            return None;
        }
        Some((self.generation, self.verifier.term(device_id)?))
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

    fn term(&self, device_id: &str) -> Option<Term> {
        (**self).term(device_id)
    }
}

/// Verifies ECDSA P-256/SHA-256 signatures (ADR-0028) against the public keys of trusted devices. Keys are uncompressed
/// SEC1 points (65 bytes); signatures are fixed 64 bytes (r || s). An unknown device never verifies.
#[derive(Default)]
pub struct P256Verifier {
    keys: std::collections::HashMap<String, Vec<u8>>,
    /// ADR-0061: devices another device endorsed are trusted until then (milliseconds since the epoch); the others have no end.
    expires: std::collections::HashMap<String, u64>,
    /// When each device's trust here began (`since` in the file), for [`Verifier::term`].
    since: std::collections::HashMap<String, u64>,
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
        self.expires.remove(device_id);
        self.since.remove(device_id);
    }
}

/// Name of the file in the node's data directory that lists the devices whose signatures this node accepts.
pub const TRUSTED_DEVICES_FILE: &str = "trusted-devices";

impl P256Verifier {
    /// Reads the trusted-devices file: one `<device id> <public key as 130 hex digits>` per line, with ADR-0061's optional
    /// `expires=<ms>` and `via=<device>` after the key (`crate::device_trust`); blank lines and `#` comments are skipped. A missing
    /// file trusts nothing. Lines that do not parse are skipped (the count of those is returned) rather than trusted: a typo must
    /// never widen trust.
    pub fn load(path: &std::path::Path) -> (Self, usize) {
        let mut v = Self::new();
        let (entries, skipped) = crate::device_trust::read_entries(path);
        for e in &entries {
            // An endorsed device lasts no longer than its endorsers up the chain; a broken chain trusts nothing.
            let Some(end) = crate::device_trust::effective_expiry(&entries, e) else {
                continue;
            };
            if v.trust(&e.id, &e.key) {
                if let Some(at) = end {
                    v.expires.insert(e.id.clone(), at);
                }
                if let Some(at) = e.since {
                    v.since.insert(e.id.clone(), at);
                }
            }
        }
        (v, skipped)
    }
}

/// Adds (or replaces) a device in the trusted-devices file. This is the node owner's act on the node itself: the
/// hub has no way to call it, which is what keeps approvals from being widened remotely (ADR-0020). Returns the devices whose
/// revocations of it this overrides (ADR-0081, [`crate::device_trust::confirm`]).
pub fn trust_device(
    path: &std::path::Path,
    device_id: &str,
    key_hex: &str,
) -> Result<Vec<String>, String> {
    let key = crate::device_trust::from_hex(key_hex).ok_or("the key is not hexadecimal")?;
    crate::device_trust::confirm(path, device_id, &key)
}

impl Verifier for P256Verifier {
    fn verify(&self, device_id: &str, message: &[u8], signature: &[u8]) -> bool {
        let Some(key) = self.keys.get(device_id) else {
            return false;
        };
        // ADR-0061: an endorsed device whose trust ran out is no longer trusted, even before the file changes.
        if self
            .expires
            .get(device_id)
            .is_some_and(|at| crate::grants::now_ms() >= *at)
        {
            return false;
        }
        ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_FIXED, key)
            .verify(message, signature)
            .is_ok()
    }

    fn term(&self, device_id: &str) -> Option<Term> {
        self.keys.contains_key(device_id).then(|| Term {
            since: self.since.get(device_id).copied(),
            until: self.expires.get(device_id).copied(),
        })
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
    /// What each trusted device may answer (ADR-0049).
    limits: std::sync::Mutex<Option<std::sync::Arc<crate::limits::DeviceLimits>>>,
    /// Requests a late decision took out of the queue (refused as expired before the sweep saw them): the next
    /// [`Self::expire`] still tells their agents, which would otherwise wait for an answer forever.
    late: std::sync::Mutex<Vec<(String, agora_core::AgentId, String)>>,
    /// ADR-0110 decision 6 (security review of PR #1190, S2): requests whose answers are being given to their adapters, so what is
    /// recorded is what reached the agent: [`Self::flight_begin`].
    flights: std::sync::Mutex<Flights>,
    /// ADR-0057: live helper grants, kept here beside the approvals that made them so grant listing and revocation (which only
    /// have the approvals) see them too.
    helper_grants: std::sync::Mutex<Vec<LiveHelper>>,
    /// ADR-0061: renews an endorsed device's trust each time its signature is accepted here.
    devices: std::sync::Mutex<Option<std::sync::Arc<crate::device_trust::DeviceTrust>>>,
    /// ADR-0090 Amendment A: what each waiting `agent.skip_permissions` request will apply, as the node rendered it.
    skips: std::sync::Mutex<SkipState>,
}

/// ADR-0110 decision 6 (security review of PR #1190, S2): the answer calls of one request that have not ended. A request is recorded
/// once: with the first answer an adapter took (made at once or late), or as cancelled only when no call that may still deliver one
/// is running. An end owed while one runs (an answer the adapter refused, an expiry, the agent's end) waits for it.
#[derive(Debug, Default)]
struct Flight {
    /// Calls made and not ended (a call followed after the node's wait counts until its thread ends).
    calls: u32,
    /// One of them is followed after the node's wait ran out (at most one per request, N1).
    watched: bool,
    /// The request was recorded.
    settled: bool,
    /// It ended without an answer reaching the agent; recorded as cancelled once no call runs.
    owed: bool,
}

/// S2: the requests with answer calls running, and (re-review of PR #1190, R2) the ids of those recorded lately, so a request whose
/// entry is gone is not read as never recorded.
#[derive(Debug, Default)]
struct Flights {
    map: std::collections::HashMap<String, Flight>,
    recorded: VecDeque<String>,
}

/// How many recorded request ids [`Flights`] remembers.
const RECORDED_MEMORY: usize = 1024;

impl Flights {
    /// Drops the entry of `id` once nothing runs and nothing is owed; a recorded one is remembered.
    fn tidy(&mut self, id: &str) {
        let Some(e) = self.map.get(id) else { return };
        if e.calls == 0 && (e.settled || !e.owed) {
            if e.settled {
                self.recorded.push_back(id.to_string());
                while self.recorded.len() > RECORDED_MEMORY {
                    self.recorded.pop_front();
                }
            }
            self.map.remove(id);
        }
    }
}

/// At most this many `agent.skip_permissions` requests wait on a node at once (a node has few agents a person switches by hand).
pub const MAX_SKIP_PENDING: usize = 8;
/// At most this many are raised per [`SKIP_WINDOW_MS`] on a node, so a client that keeps asking cannot bury the owner in cards.
pub const MAX_SKIP_PER_WINDOW: usize = 6;
pub const SKIP_WINDOW_MS: u64 = 60_000;
/// One agent may have this many cards waiting, and raise this many per [`SKIP_WINDOW_MS`], whatever else the node holds: one agent's flood
/// never keeps another agent's card from being raised.
pub const MAX_SKIP_PER_AGENT: usize = 2;
pub const MAX_SKIP_PER_AGENT_WINDOW: usize = 3;
/// Longest option id or value a card is raised for (bytes), and the longest agent kind, name and folder shown (characters).
pub const MAX_SKIP_ID_BYTES: usize = 128;
const MAX_KIND_CHARS: usize = 64;
const MAX_FOLDER_CHARS: usize = 256;
/// How long a skip request waits.
const SKIP_TTL_MS: u64 = 10 * 60 * 1000;

/// Which switch of a running agent a skip request is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipKind {
    /// `AgentSetMode`: the value is a mode id.
    Mode,
    /// `AgentSetConfig`: the option is a config id.
    Config,
}

impl SkipKind {
    fn word(self) -> &'static str {
        match self {
            Self::Mode => "mode",
            Self::Config => "config",
        }
    }
}

/// What a waiting skip request will apply and what the card says: made only by the supervisor, from what the agent reported and the
/// node's own judgement, never from a client's words. The decision applies exactly this, whatever it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkipAsk {
    pub agent: agora_core::AgentId,
    pub agent_kind: String,
    /// The agent's folder on this machine, as the node knows it.
    pub folder: String,
    pub kind: SkipKind,
    /// For a mode the literal `mode`, otherwise the config id (what is signed as `option_id`).
    pub option_id: String,
    /// The agent's own name for the option (untrusted text, bounded by [`Self::detail`]); empty for a mode.
    pub option_name: String,
    /// The mode id or the config value.
    pub value: String,
    pub value_name: String,
    /// Which run of the agent's adapter it was in (`AdapterRich::epoch`); a decision after another one began applies nothing.
    pub epoch: u64,
}

const MAX_NAME_CHARS: usize = 64;

/// A character that must never reach a card: a control character (Cc), a format character (Cf: bidirectional controls and isolates,
/// zero-width characters, the byte order mark, tags) or a line or paragraph separator.
fn is_hidden(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{ad}' | '\u{600}'..='\u{605}' | '\u{61c}' | '\u{6dd}' | '\u{70f}' | '\u{180e}'
            | '\u{200b}'..='\u{200f}' | '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}' | '\u{110bd}'
            | '\u{e0001}' | '\u{e0020}'..='\u{e007f}')
}

/// Untrusted text for a card: hidden characters removed, at most `max` characters, and a cut is marked with `…` (inside the bound).
pub(crate) fn card_text(s: &str, max: usize) -> String {
    let clean: Vec<char> = s.chars().filter(|c| !is_hidden(*c)).collect();
    if clean.len() <= max {
        return clean.into_iter().collect();
    }
    clean
        .into_iter()
        .take(max.saturating_sub(1))
        .chain(['\u{2026}'])
        .collect()
}

/// Is `s` fit to be signed as it is (an option id or a value): not empty, no hidden character, within [`MAX_SKIP_ID_BYTES`]? What is
/// signed is what the card shows, so such a string is refused, never altered.
fn signable(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_SKIP_ID_BYTES && !s.chars().any(is_hidden)
}

impl SkipAsk {
    /// The request's `detail`: the node's own rendering (docs/specs/protocol.md), which the decision's signature covers by hash.
    pub fn detail(&self, node: &str) -> String {
        serde_json::json!({
            "node": node,
            "agent": self.agent.as_str(),
            "agent_kind": card_text(&self.agent_kind, MAX_KIND_CHARS),
            "folder": card_text(&self.folder, MAX_FOLDER_CHARS),
            "option_type": self.kind.word(),
            "option": self.option_id,
            "option_name": card_text(&self.option_name, MAX_NAME_CHARS),
            "value": self.value,
            "value_name": card_text(&self.value_name, MAX_NAME_CHARS),
            // The node's own judgement (its classifier), never the agent's word: every card is for a choice the node judged as
            // making the agent act without asking.
            "skips": true,
            "scope": "session",
        })
        .to_string()
    }

    fn binding(&self) -> SkipBinding {
        SkipBinding {
            agent: self.agent.as_str().to_string(),
            option: self.option_id.clone(),
            value: self.value.clone(),
        }
    }

    fn same_option(&self, agent: &agora_core::AgentId, kind: SkipKind, option_id: &str) -> bool {
        &self.agent == agent && self.kind == kind && self.option_id == option_id
    }
}

#[derive(Default)]
struct SkipState {
    asks: std::collections::HashMap<String, SkipAsk>,
    /// When the requests of the last [`SKIP_WINDOW_MS`] were raised.
    raised: VecDeque<(u64, String)>,
}

/// Why a skip request was not raised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipRefusal {
    /// [`MAX_SKIP_PENDING`] already wait.
    TooManyWaiting,
    /// [`MAX_SKIP_PER_WINDOW`] were raised in the last minute.
    TooOften,
    /// This agent already has [`MAX_SKIP_PER_AGENT`] cards waiting.
    AgentTooMany,
    /// This agent raised [`MAX_SKIP_PER_AGENT_WINDOW`] cards in the last minute.
    AgentTooOften,
    /// The option id or the value is empty, longer than [`MAX_SKIP_ID_BYTES`] or holds a hidden character: no card, `agent.option_unknown`.
    Unsignable,
    /// No safe id could be made.
    NoId,
}

/// A skip request as [`NodeApprovals::request_skip`] gives it back.
#[derive(Debug, Clone)]
pub struct SkipRaised {
    pub pending: Pending,
    /// `false`: the same request (agent, option and value) was already waiting and this is it; nothing new is announced.
    pub fresh: bool,
    /// Requests this one replaced (the same agent and option, another value): announce each as withdrawn.
    pub superseded: Vec<String>,
}

/// "This agent may start up to `remaining` more helpers of `kind` in `folder`" (ADR-0057), from a signed decision. It lives in memory
/// and ends when the asking agent exits or is stopped, when it is revoked, or when its signer no longer vouches for it (audit F290).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperGrant {
    /// The approval id that carried it.
    pub id: String,
    pub agent: agora_core::AgentId,
    pub kind: String,
    pub folder: String,
    /// Whether it covers starts in a worktree of the folder (it covers only what the user approved: one or the other).
    pub worktree: bool,
    pub remaining: u32,
    pub helpers_may_start: bool,
    pub signer_device_id: String,
}

/// A live helper grant, with what it takes to check again at each use that its signer still vouches for it (audit F290): the
/// decision it came with as signed, and the signer's term of trust when it was made.
#[derive(Clone)]
struct LiveHelper {
    grant: HelperGrant,
    message: Vec<u8>,
    signature: Vec<u8>,
    since: Option<u64>,
}

/// A decision on a request, as [`NodeApprovals::resolve_request`] gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub agent: agora_core::AgentId,
    pub adapter_approval_id: String,
    pub verdict: Verdict,
    pub request: Pending,
    /// For a [`SKIP_ACTION`] request: what the node rendered and applies when it was approved.
    pub skip: Option<SkipAsk>,
    /// ADR-0101: the verified decision is "always" (only ever with [`Verdict::Approved`]).
    pub always: bool,
}

impl NodeApprovals {
    /// ADR-0057: records a helper grant made from a verified decision: `message` is what its signer signed
    /// ([`start_signed_message`]) and `signature` the signature. A signer no longer trusted gives no grant.
    pub fn add_helper_grant(&self, g: HelperGrant, message: Vec<u8>, signature: Vec<u8>) {
        if g.remaining == 0 {
            return;
        }
        let Some((_, term)) = self.vouches(&g.signer_device_id, &message, &signature) else {
            return;
        };
        self.hg().push(LiveHelper {
            grant: g,
            message,
            signature,
            since: term.since,
        });
    }

    /// Does a helper grant's signer still vouch for it (audit F290): its decision verifies under the devices trusted now, in the
    /// term it was made in, before that term ends, and the device may still answer for the agent (ADR-0049)? Renews nothing.
    fn helper_vouched(&self, h: &LiveHelper, now_ms: u64) -> bool {
        let g = &h.grant;
        let Some((_, term)) = self.vouches(&g.signer_device_id, &h.message, &h.signature) else {
            return false;
        };
        let limits = self
            .limits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        term.since == h.since
            && term.until.is_none_or(|u| now_ms < u)
            && limits.is_none_or(|l| l.allows_agent(&g.signer_device_id, g.agent.as_str()))
    }

    /// Ends for good every helper grant `which` selects whose signer no longer vouches for it (audit F290). The trusted-devices file
    /// is read again first when it changed, so a device removed by another process counts at once.
    fn recheck_helper_grants(&self, which: impl Fn(&HelperGrant) -> bool) {
        let check: Vec<LiveHelper> = self
            .hg()
            .iter()
            .filter(|h| which(&h.grant))
            .cloned()
            .collect();
        if check.is_empty() {
            return;
        }
        self.reload_trust_if_changed();
        let now = crate::grants::now_ms();
        // Checked without holding the grants: the limits may ask the supervisor for an agent's project.
        let ended: Vec<String> = check
            .into_iter()
            .filter(|h| !self.helper_vouched(h, now))
            .map(|h| h.grant.id)
            .collect();
        if !ended.is_empty() {
            self.hg().retain(|h| !ended.contains(&h.grant.id));
        }
    }

    /// Uses one start of a live grant of `agent` for `kind` in `folder`: `Some(helpers_may_start)` if one covered it.
    pub fn use_helper_grant(
        &self,
        agent: &agora_core::AgentId,
        kind: &str,
        folder: &str,
        worktree: bool,
    ) -> Option<bool> {
        let covers = |g: &HelperGrant| {
            g.agent == *agent
                && g.kind == kind
                && g.folder == folder
                && g.worktree == worktree
                && g.remaining > 0
        };
        self.recheck_helper_grants(covers);
        let mut grants = self.hg();
        let g = &mut grants.iter_mut().find(|h| covers(&h.grant))?.grant;
        g.remaining -= 1;
        let nested = g.helpers_may_start;
        grants.retain(|h| h.grant.remaining > 0);
        Some(nested)
    }

    /// Every live helper grant, as `GrantList` shows them (one whose signer no longer vouches for it has ended).
    pub fn helper_grants(&self) -> Vec<HelperGrant> {
        self.recheck_helper_grants(|_| true);
        self.hg().iter().map(|h| h.grant.clone()).collect()
    }

    /// Ends one helper grant (`GrantRevoke`); `false` if there was none with that id.
    pub fn revoke_helper_grant(&self, id: &str) -> bool {
        let mut grants = self.hg();
        let before = grants.len();
        grants.retain(|h| h.grant.id != id);
        grants.len() != before
    }

    /// The agent exited or was stopped: its grants end with it.
    pub fn end_helper_grants(&self, agent: &agora_core::AgentId) {
        self.hg().retain(|h| h.grant.agent != *agent);
    }

    fn hg(&self) -> std::sync::MutexGuard<'_, Vec<LiveHelper>> {
        self.helper_grants.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// An agent asks to start another (ADR-0057). Returns the pending request to announce, or `None` if no safe id could be made.
    pub fn request_start(
        &self,
        agent: &agora_core::AgentId,
        detail: &str,
        now_ms: u64,
    ) -> Option<Pending> {
        let p = self
            .q()
            .request(agent.as_str(), START_ACTION, detail, now_ms, 0)?;
        self.r()
            .insert(p.id.clone(), (agent.clone(), String::new()));
        Some(p)
    }

    /// Limits which requests each device may answer (`device.<id>.projects` in the policy file).
    pub fn set_limits(&self, limits: std::sync::Arc<crate::limits::DeviceLimits>) {
        *self.limits.lock().unwrap_or_else(|e| e.into_inner()) = Some(limits);
    }

    pub fn new(node_id: &str, verifier: Box<dyn Verifier + Send>) -> Self {
        Self {
            queue: std::sync::Mutex::new(Approvals::new(node_id, verifier)),
            routes: std::sync::Mutex::new(std::collections::HashMap::new()),
            trust_file: std::sync::Mutex::new(None),
            limits: std::sync::Mutex::new(None),
            late: std::sync::Mutex::new(Vec::new()),
            flights: std::sync::Mutex::new(Flights::default()),
            helper_grants: std::sync::Mutex::new(Vec::new()),
            devices: std::sync::Mutex::new(None),
            skips: std::sync::Mutex::new(SkipState::default()),
        }
    }

    /// ADR-0061: the trust store whose endorsed devices are renewed by use.
    pub fn set_device_trust(&self, devices: std::sync::Arc<crate::device_trust::DeviceTrust>) {
        *self.devices.lock().unwrap_or_else(|e| e.into_inner()) = Some(devices);
    }

    /// A trusted device's signature was just accepted: an endorsed device's trust runs 90 days from now.
    fn used(&self, device_id: &str) {
        let devices = self
            .devices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(d) = devices {
            d.renew(device_id, crate::grants::now_ms());
        }
    }

    fn l(&self) -> std::sync::MutexGuard<'_, Vec<(String, agora_core::AgentId, String)>> {
        self.late.lock().unwrap_or_else(|e| e.into_inner())
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

    /// Does `device_id` (as the trusted-devices file lists it now) vouch for `message`? Used for grants (ADR-0041).
    pub fn verify_device(&self, device_id: &str, message: &[u8], signature: &[u8]) -> bool {
        self.verify_device_term(device_id, message, signature)
            .is_some()
    }

    /// Like [`Self::verify_device`] (a use: an endorsed device's trust is renewed), with the trust generation the signature was
    /// verified under and the device's term, so a verdict that lasts (a grant) can be checked again when trust changes (audit F270).
    pub fn verify_device_term(
        &self,
        device_id: &str,
        message: &[u8],
        signature: &[u8],
    ) -> Option<(u64, Term)> {
        self.reload_trust_if_changed();
        let verdict = self.q().verify_term(device_id, message, signature);
        if verdict.is_some() {
            self.used(device_id);
        }
        verdict
    }

    /// The trust generation now: it moves whenever the trusted devices change (the file is read again when it changed, so a device
    /// the owner removed in another process counts at once). No signature is checked and nothing is renewed or written.
    pub fn trust_generation(&self) -> u64 {
        self.reload_trust_if_changed();
        self.q().generation
    }

    /// Does `device_id` still vouch for `message` under the trust held now, and in which term? A check, not a use: unlike
    /// [`Self::verify_device_term`] it neither reads the file (see [`Self::trust_generation`]) nor renews the device's trust.
    pub fn vouches(
        &self,
        device_id: &str,
        message: &[u8],
        signature: &[u8],
    ) -> Option<(u64, Term)> {
        self.q().verify_term(device_id, message, signature)
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
        self.request_offered(
            agent,
            adapter_approval_id,
            action,
            detail,
            now_ms,
            Offer::once_only(),
        )
    }

    /// ADR-0101: [`Self::request`] offering the answers in `offer` (see [`raise_for`]).
    pub fn request_offered(
        &self,
        agent: &agora_core::AgentId,
        adapter_approval_id: &str,
        action: &str,
        detail: &str,
        now_ms: u64,
        offer: Offer,
    ) -> Option<Pending> {
        // The `agent.*` actions are the node's own (`agent.start`, `agent.skip_permissions`): their cards come from the node's own
        // records and their decisions are applied from them. An adapter that names one gets no card and nothing is consumed; the
        // caller answers it "denied" at once. An adapter's own actions (`run_command`, `write_file`) are not in that namespace.
        if is_reserved_action(action) {
            return None;
        }
        let p = self.raise_adapter(agent, action, detail, now_ms, offer)?;
        self.route_adapter(&p, agent, adapter_approval_id);
        Some(p)
    }

    /// The first half of [`Self::request_offered`]: the request is in the queue, without its route back to the adapter yet. Another
    /// thread (a cancelled turn) can run between the halves; tests use them apart to make that interleaving.
    fn raise_adapter(
        &self,
        agent: &agora_core::AgentId,
        action: &str,
        detail: &str,
        now_ms: u64,
        offer: Offer,
    ) -> Option<Pending> {
        self.q()
            .request_offered(agent.as_str(), action, detail, now_ms, offer)
    }

    /// The second half of [`Self::request_offered`]: the route back to the adapter, which makes the request fully registered.
    fn route_adapter(&self, p: &Pending, agent: &agora_core::AgentId, adapter_approval_id: &str) {
        self.r().insert(
            p.id.clone(),
            (agent.clone(), adapter_approval_id.to_string()),
        );
    }

    fn sk(&self) -> std::sync::MutexGuard<'_, SkipState> {
        self.skips.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// ADR-0090 Amendment A: raises the request to switch a running agent as `ask` says, or finds the one that already waits for the
    /// same agent, option and value (one card each). Another value of the same option replaces the waiting one. The caller announces
    /// `pending` when `fresh` and each of `superseded` as withdrawn.
    pub fn request_skip(
        &self,
        node: &str,
        ask: SkipAsk,
        now_ms: u64,
    ) -> Result<SkipRaised, SkipRefusal> {
        if !signable(&ask.option_id) || !signable(&ask.value) {
            return Err(SkipRefusal::Unsignable);
        }
        let mut st = self.sk();
        // A request that ran out of time without being swept is not one that waits.
        let alive: std::collections::HashSet<String> =
            self.q().pending().map(|p| p.id.clone()).collect();
        st.asks.retain(|id, _| alive.contains(id));
        let same: Vec<(String, bool)> = st
            .asks
            .iter()
            .filter(|(_, a)| a.same_option(&ask.agent, ask.kind, &ask.option_id))
            .map(|(id, a)| (id.clone(), a.value == ask.value))
            .collect();
        if let Some((id, _)) = same.iter().find(|(_, equal)| *equal) {
            let pending = self.q().pending().find(|p| &p.id == id).cloned();
            if let Some(pending) = pending {
                return Ok(SkipRaised {
                    pending,
                    fresh: false,
                    superseded: Vec::new(),
                });
            }
        }
        // A replacement counts like any other card here (it is withdrawn only once the new one exists), and in the rate below.
        if st.asks.len() >= MAX_SKIP_PENDING {
            return Err(SkipRefusal::TooManyWaiting);
        }
        if st.asks.values().filter(|a| a.agent == ask.agent).count() >= MAX_SKIP_PER_AGENT {
            return Err(SkipRefusal::AgentTooMany);
        }
        while st
            .raised
            .front()
            .is_some_and(|(t, _)| now_ms.saturating_sub(*t) >= SKIP_WINDOW_MS)
        {
            st.raised.pop_front();
        }
        if st.raised.len() >= MAX_SKIP_PER_WINDOW {
            return Err(SkipRefusal::TooOften);
        }
        if st
            .raised
            .iter()
            .filter(|(_, a)| a == ask.agent.as_str())
            .count()
            >= MAX_SKIP_PER_AGENT_WINDOW
        {
            return Err(SkipRefusal::AgentTooOften);
        }
        let detail = ask.detail(node);
        let pending = self
            .q()
            .request_bound(
                ask.agent.as_str(),
                SKIP_ACTION,
                &detail,
                now_ms,
                SKIP_TTL_MS,
                Some(ask.binding()),
            )
            .ok_or(SkipRefusal::NoId)?;
        self.r()
            .insert(pending.id.clone(), (ask.agent.clone(), String::new()));
        let mut superseded = Vec::new();
        for (id, _) in same.iter().filter(|(_, equal)| !*equal) {
            st.asks.remove(id);
            self.withdraw(id);
            superseded.push(id.clone());
        }
        st.raised
            .push_back((now_ms, ask.agent.as_str().to_string()));
        st.asks.insert(pending.id.clone(), ask);
        Ok(SkipRaised {
            pending,
            fresh: true,
            superseded,
        })
    }

    /// Withdraws the waiting skip requests of `agent` (all, or those for one option) and returns their ids: the agent ended, or the
    /// option was set by another command, so nothing may be applied for them later. The caller announces each as withdrawn.
    pub fn withdraw_skips(
        &self,
        agent: &agora_core::AgentId,
        only: Option<(SkipKind, &str)>,
    ) -> Vec<String> {
        let ids: Vec<String> = {
            let mut st = self.sk();
            let ids: Vec<String> = st
                .asks
                .iter()
                .filter(|(_, a)| {
                    &a.agent == agent
                        && only.is_none_or(|(kind, option)| a.same_option(agent, kind, option))
                })
                .map(|(id, _)| id.clone())
                .collect();
            for id in &ids {
                st.asks.remove(id);
            }
            ids
        };
        for id in &ids {
            self.withdraw(id);
        }
        ids
    }

    /// Forgets the record of a skip request that ran out of time (the sweep found it); `true` when it was one, so its answer is
    /// not sent to an adapter (it has none).
    pub fn forget_skip(&self, id: &str) -> bool {
        self.sk().asks.remove(id).is_some()
    }

    /// A signed decision arrived. On success, the adapter's own approval id and agent to answer, and the verdict.
    pub fn resolve(
        &self,
        r: &ApprovalResolve,
        now_ms: u64,
    ) -> Result<(agora_core::AgentId, String, Verdict), ApprovalError> {
        self.resolve_request(r, now_ms)
            .map(|x| (x.agent, x.adapter_approval_id, x.verdict))
    }

    /// Like [`Self::resolve`], with the request that was decided (its action and detail). Never "always" (see
    /// [`Self::resolve_decided`]).
    pub fn resolve_request(
        &self,
        r: &ApprovalResolve,
        now_ms: u64,
    ) -> Result<Resolution, ApprovalError> {
        self.resolve_decided(r, now_ms, Decide::default())
    }

    /// ADR-0101: the agent a waiting request is routed back to (the one whose policy decides it), if any.
    pub fn agent_of(&self, approval_id: &str) -> Option<agora_core::AgentId> {
        self.r().get(approval_id).map(|(agent, _)| agent.clone())
    }

    /// [`Self::resolve_request`] under the policy `decide`, read by the caller for [`Self::agent_of`] before this is called (the
    /// supervisor's lock is never taken under this queue's).
    pub fn resolve_decided(
        &self,
        r: &ApprovalResolve,
        now_ms: u64,
        decide: Decide,
    ) -> Result<Resolution, ApprovalError> {
        self.reload_trust_if_changed();
        // Before anything is consumed: a device limited to other projects cannot answer this agent's request. The ids are 128
        // random bits, so only someone who already knows one learns that it exists.
        let limits = self
            .limits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let (Some(limits), Some((agent, _))) = (limits, self.r().get(&r.approval_id).cloned()) {
            if !limits.allows_agent(&r.signer_device_id, agent.as_str()) {
                return Err(ApprovalError::Forbidden);
            }
        }
        let resolved = self.q().resolve_decided(r, now_ms, decide);
        let (request, verdict) = match resolved {
            Ok(v) => v,
            Err(ApprovalError::Expired) => {
                // The queue dropped the request, so the sweep will not see it: hand its route to the next sweep instead.
                if let Some((agent, id)) = self.r().remove(&r.approval_id) {
                    self.l().push((r.approval_id.clone(), agent, id));
                }
                return Err(ApprovalError::Expired);
            }
            Err(e) => return Err(e),
        };
        self.used(&r.signer_device_id);
        let route = self.r().remove(&r.approval_id);
        let skip = if request.action == SKIP_ACTION {
            self.sk().asks.remove(&r.approval_id)
        } else {
            None
        };
        route
            .map(|(agent, adapter_approval_id)| Resolution {
                agent,
                adapter_approval_id,
                verdict,
                request,
                skip,
                always: r.scope == agora_protocol::v1::ApprovalScope::Always as i32,
            })
            .ok_or(ApprovalError::Unknown)
    }

    /// Requests that ran out of time: the agent behind each is told "denied". Returns `(node approval id, agent,
    /// adapter approval id)`.
    pub fn expire(&self, now_ms: u64) -> Vec<(String, agora_core::AgentId, String)> {
        let gone = self.q().expire(now_ms);
        let mut out = std::mem::take(&mut *self.l());
        let mut routes = self.r();
        out.extend(
            gone.into_iter()
                .filter_map(|p| routes.remove(&p.id).map(|(a, id)| (p.id, a, id))),
        );
        out
    }

    /// An agent stopped or died, or its turn was cancelled: nothing stays waiting on it. Returns the ids of the requests that were
    /// withdrawn (ADR-0090: their entries in the conversation are shown as cancelled).
    pub fn cancel_agent(&self, agent: &agora_core::AgentId) -> Vec<String> {
        let mut withdrawn = Vec::new();
        self.l().retain(|(id, a, _)| {
            let keep = a != agent;
            if !keep {
                withdrawn.push(id.clone());
            }
            keep
        });
        let gone = self.q().cancel_agent(agent.as_str());
        let mut routes = self.r();
        for p in gone {
            routes.remove(&p.id);
            withdrawn.push(p.id);
        }
        drop(routes);
        self.recorded_by_caller(&withdrawn);
        withdrawn
    }

    /// S2: the caller records these requests (as cancelled) itself: an answer call of one that is still followed records nothing
    /// when it ends.
    fn recorded_by_caller(&self, ids: &[String]) {
        let mut f = self.fl();
        for id in ids {
            if let Some(e) = f.map.get_mut(id) {
                e.settled = true;
            }
        }
    }

    /// ADR-0090: the agent's turn was cancelled: the permission requests its adapter asked for (the ones that have a route back to the
    /// adapter) are withdrawn and their ids returned. Requests of another kind (an `agent.start` the agent waits for) stay: the agent
    /// is still alive and still waits for them.
    pub fn cancel_adapter_requests(&self, agent: &agora_core::AgentId) -> Vec<String> {
        // Fully registered adapter requests only: routed back to this agent's adapter (a request whose route is not recorded yet is
        // still being raised on another thread, and withdrawn now it would come back as an orphan route and an announced request that
        // is gone), and never the node's own `agent.*` requests, which also have routes (`request_start`). The routes are let go
        // before the queue is asked: neither lock is held inside the other.
        let routed: Vec<String> = self
            .r()
            .iter()
            .filter(|(_, (a, _))| a == agent)
            .map(|(id, _)| id.clone())
            .collect();
        let ids: Vec<String> = {
            let q = self.q();
            routed
                .into_iter()
                .filter(|id| {
                    q.pending().any(|p| {
                        &p.id == id
                            && p.agent_id == agent.as_str()
                            && !is_reserved_action(&p.action)
                    })
                })
                .collect()
        };
        for id in &ids {
            self.withdraw(id);
        }
        self.recorded_by_caller(&ids);
        ids
    }

    fn fl(&self) -> std::sync::MutexGuard<'_, Flights> {
        self.flights.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// S2: an answer call for request `id` is about to be made. True when it may be followed after the node's wait (no other call
    /// of the request is).
    pub fn flight_begin(&self, id: &str) -> bool {
        let mut f = self.fl();
        let e = f.map.entry(id.to_string()).or_default();
        e.calls += 1;
        !e.watched
    }

    /// S2: the call is followed after the node's wait ran out ([`AdapterCall::run_keeping_late`]'s `give_up`).
    ///
    /// [`AdapterCall::run_keeping_late`]: crate::supervisor::AdapterCall::run_keeping_late
    pub fn flight_watched(&self, id: &str) {
        if let Some(e) = self.fl().map.get_mut(id) {
            e.watched = true;
        }
    }

    /// S2: whether a call of request `id` is followed after the node's wait.
    pub fn flight_is_watched(&self, id: &str) -> bool {
        self.fl().map.get(id).is_some_and(|e| e.watched)
    }

    /// S2: a call of `id` took its answer (`late`: the followed one). True when this answer is the one to record (nothing was).
    pub fn flight_delivered(&self, id: &str, late: bool) -> bool {
        self.flight_end(id, late, |e| {
            let first = !e.settled;
            e.settled = true;
            first
        })
    }

    /// S2: a call of `id` ended and its answer did not reach the agent. `owed`: the request is over (the adapter refused the answer,
    /// as one it no longer holds); a followed call that ran out (`late`) owes nothing by itself. True when the request is to be
    /// recorded as cancelled now: something owed it and no call that may still deliver an answer runs.
    pub fn flight_undelivered(&self, id: &str, owed: bool, late: bool) -> bool {
        self.flight_end(id, late, |e| {
            e.owed |= owed;
            Self::owed_now(e)
        })
    }

    /// S2: request `id` ended without a call (it expired, or its agent's run ended while it was kept). True when it is to be
    /// recorded as cancelled now; false when a call that may still deliver an answer runs (that call records it) or it was recorded.
    pub fn flight_ended(&self, id: &str) -> bool {
        let mut f = self.fl();
        let Some(e) = f.map.get_mut(id) else {
            // R2: no call runs; recorded already only if remembered so.
            return !f.recorded.iter().any(|r| r == id);
        };
        e.owed = true;
        let now = Self::owed_now(e);
        f.tidy(id);
        now
    }

    fn owed_now(e: &mut Flight) -> bool {
        if e.settled || !e.owed || e.calls > 0 {
            return false;
        }
        e.settled = true;
        true
    }

    fn flight_end(&self, id: &str, late: bool, f: impl FnOnce(&mut Flight) -> bool) -> bool {
        let mut all = self.fl();
        let Some(e) = all.map.get_mut(id) else {
            return false;
        };
        e.calls = e.calls.saturating_sub(1);
        if late {
            e.watched = false;
        }
        let out = f(e);
        all.tidy(id);
        out
    }

    /// Re-review of PR #1190, R3: [`Self::restore`], decided under the lock the answer calls record under, so a request an answer
    /// was recorded for meanwhile is never put back (announced as waiting again, or swept as expired a second time). `ends_call`: the
    /// caller's own call of the request ends here (one not followed). `None` when it was recorded; else whether it was put back.
    pub fn restore_unless_recorded(
        &self,
        resolution: &Resolution,
        now_ms: u64,
        ends_call: bool,
    ) -> Option<bool> {
        let id = resolution.request.id.as_str();
        // Lock order: the flights, then the queue, routes and sweep list (nothing takes the flights while holding one of those).
        let mut f = self.fl();
        let recorded = match f.map.get_mut(id) {
            Some(e) => {
                if ends_call {
                    e.calls = e.calls.saturating_sub(1);
                }
                e.settled
            }
            None => f.recorded.iter().any(|r| r == id),
        };
        let out = (!recorded).then(|| self.restore(resolution, now_ms));
        f.tidy(id);
        out
    }

    /// ADR-0110 decision 6: puts back the request of `resolution`, whose verified answer the adapter did not take within the node's
    /// wait, so the card stays pending and can be answered again. The route goes back before the request (a decision takes the
    /// request first and its route after). A request that ran out of time meanwhile is handed to the next sweep instead, as one
    /// that expires under a decision is. True when it was put back.
    pub fn restore(&self, resolution: &Resolution, now_ms: u64) -> bool {
        let id = resolution.request.id.clone();
        let route = (
            resolution.agent.clone(),
            resolution.adapter_approval_id.clone(),
        );
        if now_ms >= resolution.request.expires_ms {
            self.l().push((id, route.0, route.1));
            return false;
        }
        self.r().insert(id, route);
        self.q().restore(resolution.request.clone());
        true
    }

    /// ADR-0110 decision 6: takes request `id` out if it still waits, for the one answer that settles it (a late answer from the
    /// adapter, or a check that its agent is gone). False when it was already answered, withdrawn or expired: then nothing is done.
    pub fn take_pending(&self, id: &str) -> bool {
        let taken = self.q().withdraw(id).is_some();
        // R2: also when it was not in the queue (handed to the sweep as expired while its answer call ran): the sweep must not end
        // a request that was settled.
        self.r().remove(id);
        self.l().retain(|(i, _, _)| i != id);
        taken
    }

    /// Withdraws one request (ADR-0057: the agent that asked to start a helper stopped waiting).
    pub fn withdraw(&self, id: &str) {
        self.l().retain(|(i, _, _)| i != id);
        self.q().withdraw(id);
        self.r().remove(id);
    }

    pub fn pending_count(&self) -> usize {
        self.q().pending().count()
    }

    /// Every request still waiting for a decision, for the node to announce again (ADR-0054).
    pub fn waiting(&self) -> Vec<Pending> {
        self.q().pending().cloned().collect()
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
            helper_grant: 0,
            helpers_may_start: false,
            ..Default::default()
        }
    }

    fn skip_ask(agent: &str, kind: SkipKind, option: &str, value: &str) -> SkipAsk {
        SkipAsk {
            agent: agora_core::AgentId::new(agent),
            agent_kind: "claude-acp".into(),
            folder: "/work/login-fix".into(),
            kind,
            option_id: option.into(),
            option_name: "Mode".into(),
            value: value.into(),
            value_name: "Bypass Permissions".into(),
            epoch: 0,
        }
    }

    fn skip_resolve(
        n: &NodeApprovals,
        p: &Pending,
        approved: bool,
        binding: &SkipBinding,
        now: u64,
    ) -> Result<Resolution, ApprovalError> {
        n.resolve_request(
            &ApprovalResolve {
                approval_id: p.id.clone(),
                approved,
                signer_device_id: "phone".into(),
                signature: sign(
                    "s3cret",
                    &skip_signed_message("node-1", &p.id, approved, binding, &p.detail),
                ),
                helper_grant: 0,
                helpers_may_start: false,
                ..Default::default()
            },
            now,
        )
    }

    fn node_approvals() -> NodeApprovals {
        NodeApprovals::new("node-1", Box::new(Fake(vec![("phone", "s3cret")])))
    }

    // ---- ADR-0101: an "always" answer ----

    const ONCE: i32 = agora_protocol::v1::ApprovalScope::Once as i32;
    const ALWAYS: i32 = agora_protocol::v1::ApprovalScope::Always as i32;

    fn offer(once: bool, always: bool) -> Offer {
        Offer {
            once,
            always: always.then(|| AlwaysOption {
                label: "Always allow git status".into(),
                option_id: "aa".into(),
            }),
        }
    }

    fn offered(a: &mut Approvals<Fake>, o: Offer) -> Pending {
        a.request_offered(
            "agent-1",
            "run_command",
            "{\"command\":\"git status\"}",
            1,
            o,
        )
        .unwrap()
    }

    /// A decision signed by the phone: over the always bytes for `sign_always`, else over the v1 bytes; sent with `scope`.
    fn decision(p: &Pending, approved: bool, sign_always: bool, scope: i32) -> ApprovalResolve {
        let msg = if sign_always {
            always_signed_message("node-1", &p.id, p)
        } else {
            signed_message("node-1", &p.id, approved)
        };
        ApprovalResolve {
            approval_id: p.id.clone(),
            approved,
            signer_device_id: "phone".into(),
            signature: sign("s3cret", &msg),
            scope,
            ..Default::default()
        }
    }

    const ALLOWED: Decide = Decide {
        always: true,
        once: true,
    };

    /// The vectors every client signs by (`schema/fixtures/signing/approval-always-v1.json`, also checked by agora-protocol).
    #[test]
    fn the_shared_always_vectors_match() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/fixtures/signing/approval-always-v1.json");
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for c in v["cases"].as_array().unwrap() {
            let s = |k: &str| c[k].as_str().unwrap().to_string();
            let p = Pending {
                id: s("approval_id"),
                agent_id: s("agent_id"),
                action: s("action"),
                detail: s("detail"),
                requested_ms: 0,
                expires_ms: 0,
                binding: None,
                offer: Offer {
                    once: true,
                    always: Some(AlwaysOption {
                        label: s("always_label"),
                        option_id: s("always_option_id"),
                    }),
                },
            };
            let m = always_signed_message(&s("node"), &s("approval_id"), &p);
            let hex: String = m.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hex, s("hex"), "{}", c["name"]);
        }
    }

    #[test]
    fn an_always_answer_signed_over_its_domain_is_accepted_once() {
        let mut a = approvals();
        let p = offered(&mut a, offer(true, true));
        let (got, v) = a
            .resolve_decided(&decision(&p, true, true, ALWAYS), 2, ALLOWED)
            .unwrap();
        assert_eq!((got.id.as_str(), v), (p.id.as_str(), Verdict::Approved));
        assert_eq!(
            a.resolve_decided(&decision(&p, true, true, ALWAYS), 3, ALLOWED),
            Err(ApprovalError::Unknown),
            "single use"
        );
    }

    /// A hub that turns a single answer into "always", or "always" into a single answer, makes the node check the signature against
    /// the other domain's bytes: refused, and the card still waits (nothing spent, nothing sent).
    #[test]
    fn a_scope_changed_in_transit_fails_the_signature_both_ways() {
        let mut a = approvals();
        let p = offered(&mut a, offer(true, true));
        // ONCE signed, ALWAYS sent.
        assert_eq!(
            a.resolve_decided(&decision(&p, true, false, ALWAYS), 2, ALLOWED),
            Err(ApprovalError::BadSignature)
        );
        // ALWAYS signed, ONCE (and unset) sent.
        for scope in [ONCE, 0] {
            assert_eq!(
                a.resolve_decided(&decision(&p, true, true, scope), 2, ALLOWED),
                Err(ApprovalError::BadSignature),
                "{scope}"
            );
        }
        assert_eq!(a.pending().count(), 1, "the card still waits");
        // The honest answer still goes through.
        assert!(a
            .resolve_decided(&decision(&p, true, true, ALWAYS), 2, ALLOWED)
            .is_ok());
    }

    /// The always bytes bind what the card showed: a label, option id, action or detail the hub rewrote does not verify against what the
    /// node holds.
    #[test]
    fn an_always_signature_over_other_words_fails() {
        let mut a = approvals();
        let p = offered(&mut a, offer(true, true));
        let mut shown = p.clone();
        for change in 0..4 {
            match change {
                0 => shown.offer.always.as_mut().unwrap().label = "Allow once".into(),
                1 => shown.offer.always.as_mut().unwrap().option_id = "other".into(),
                2 => shown.action = "read_file".into(),
                _ => shown.detail = "{}".into(),
            }
            let mut r = decision(&p, true, true, ALWAYS);
            r.signature = sign("s3cret", &always_signed_message("node-1", &p.id, &shown));
            assert_eq!(
                a.resolve_decided(&r, 2, ALLOWED),
                Err(ApprovalError::BadSignature),
                "{change}"
            );
            shown = p.clone();
        }
    }

    /// "Always" is never a denial and never an answer to a node-made request (`agent.start`, `agent.skip_permissions`), and a grant
    /// means nothing on it; a value this build does not know is refused before anything else.
    #[test]
    fn always_is_refused_where_it_cannot_apply() {
        let mut a = approvals();
        let p = offered(&mut a, offer(true, true));
        let mut denied = decision(&p, false, true, ALWAYS);
        denied.signature = sign("s3cret", &always_signed_message("node-1", &p.id, &p));
        assert_eq!(
            a.resolve_decided(&denied, 2, ALLOWED),
            Err(ApprovalError::BadSignature)
        );
        let mut granted = decision(&p, true, true, ALWAYS);
        granted.helper_grant = 2;
        assert_eq!(
            a.resolve_decided(&granted, 2, ALLOWED),
            Err(ApprovalError::BadSignature)
        );
        for scope in [3, 7, -1] {
            assert_eq!(
                a.resolve_decided(&decision(&p, true, true, scope), 2, ALLOWED),
                Err(ApprovalError::InvalidScope),
                "{scope}"
            );
        }
        for action in [START_ACTION, SKIP_ACTION, "agent.other"] {
            let mut q = a
                .request_offered("agent-1", action, "{}", 1, offer(true, true))
                .unwrap();
            q.offer = offer(true, true);
            assert_eq!(
                a.resolve_decided(&decision(&q, true, true, ALWAYS), 2, ALLOWED),
                Err(ApprovalError::BadSignature),
                "{action}"
            );
        }
        assert_eq!(a.pending().count(), 4, "nothing was spent");
    }

    /// The decision is checked against the offer the node stored, never against anything it carries: "always" on a request that
    /// did not offer it, and a single Allow on one that offers only "always", are refused and leave the card waiting. A single Deny
    /// stays valid everywhere.
    #[test]
    fn an_answer_the_request_did_not_offer_is_refused_and_spends_nothing() {
        let mut a = approvals();
        let once_only = offered(&mut a, Offer::once_only());
        assert_eq!(
            a.resolve_decided(&decision(&once_only, true, true, ALWAYS), 2, ALLOWED),
            Err(ApprovalError::ScopeUnavailable)
        );
        let always_only = offered(&mut a, offer(false, true));
        for scope in [0, ONCE] {
            assert_eq!(
                a.resolve_decided(&decision(&always_only, true, false, scope), 2, ALLOWED),
                Err(ApprovalError::ScopeUnavailable),
                "{scope}"
            );
        }
        assert_eq!(a.pending().count(), 2);
        assert_eq!(
            a.resolve_decided(&decision(&always_only, false, false, ONCE), 2, ALLOWED)
                .unwrap()
                .1,
            Verdict::Denied
        );
        assert_eq!(
            a.resolve_decided(&decision(&once_only, true, false, ONCE), 2, ALLOWED)
                .unwrap()
                .1,
            Verdict::Approved
        );
    }

    /// The policy checked at decision time refuses "always" (it may differ from the one the card was raised under: an earlier node's,
    /// or another launch of the agent); the card still waits and can be allowed once. Without the policy (`resolve`), "always" is never given.
    #[test]
    fn a_policy_that_refuses_always_refuses_it_at_decision_time() {
        let mut a = approvals();
        let p = offered(&mut a, offer(true, true));
        let forbid = Decide {
            always: false,
            once: true,
        };
        assert_eq!(
            a.resolve_decided(&decision(&p, true, true, ALWAYS), 2, forbid),
            Err(ApprovalError::ScopeForbidden)
        );
        assert_eq!(
            a.resolve(&decision(&p, true, true, ALWAYS), 2),
            Err(ApprovalError::ScopeForbidden)
        );
        assert_eq!(a.pending().count(), 1);
        assert!(a
            .resolve_decided(&decision(&p, true, false, ONCE), 2, forbid)
            .is_ok());
    }

    /// Codex review of ADR-0101: an Allow through an ACP bridge from before ADR-0101 under a policy that refuses "always" is refused
    /// (that bridge may remember it); a Deny still goes through.
    #[test]
    fn an_allow_through_an_outdated_bridge_is_refused_under_a_strict_policy() {
        let mut a = approvals();
        let p = offered(&mut a, Offer::once_only());
        let legacy = Decide {
            always: false,
            once: false,
        };
        assert_eq!(
            a.resolve_decided(&decision(&p, true, false, 0), 2, legacy),
            Err(ApprovalError::AdapterOutdated)
        );
        assert_eq!(a.pending().count(), 1);
        assert_eq!(
            a.resolve_decided(&decision(&p, false, false, 0), 2, legacy)
                .unwrap()
                .1,
            Verdict::Denied
        );
    }

    /// T4: agent text never chooses "always": an adapter that dresses its request as a node-made one gets no card, whatever it offers,
    /// and the offer alone allows nothing (only a decision signed over the always bytes does).
    #[test]
    fn an_adapters_offer_never_decides_by_itself() {
        let n = node_approvals();
        let agent = agora_core::AgentId::new("agent-1");
        for action in [START_ACTION, SKIP_ACTION, "agent.anything"] {
            assert!(n
                .request_offered(&agent, "a-1", action, "{}", 1, offer(true, true))
                .is_none());
        }
        let p = n
            .request_offered(&agent, "a-2", "run_command", "ls", 1, offer(false, true))
            .unwrap();
        let mut unsigned = decision(&p, true, true, ALWAYS);
        unsigned.signature = Vec::new();
        assert_eq!(
            n.resolve_decided(&unsigned, 2, ALLOWED).unwrap_err(),
            ApprovalError::BadSignature
        );
        let r = n
            .resolve_decided(&decision(&p, true, true, ALWAYS), 2, ALLOWED)
            .unwrap();
        assert!(r.always && r.verdict == Verdict::Approved);
        assert_eq!(r.adapter_approval_id, "a-2");
    }

    /// Which policy a decision meets, from the node's policy, the adapter's declaration and whether it is the ACP bridge.
    #[test]
    fn the_policy_for_an_adapter_follows_its_declaration() {
        let d = |always, once| Decide { always, once };
        // permitted, strict, bridge
        assert_eq!(Decide::for_adapter(true, true, true), d(true, true));
        assert_eq!(Decide::for_adapter(false, true, true), d(false, true));
        // An outdated bridge: single answers as before under `allow` (the named residual), refused under a refusing policy.
        assert_eq!(Decide::for_adapter(true, false, true), d(false, true));
        assert_eq!(Decide::for_adapter(false, false, true), d(false, false));
        // An L3 adapter that does not declare it: single answers only, under any policy.
        assert_eq!(Decide::for_adapter(true, false, false), d(false, true));
        assert_eq!(Decide::for_adapter(false, false, false), d(false, true));
    }

    /// A client from before ADR-0101 sends no scope: Allow and Deny work exactly as before on a request that offers the single answer.
    #[test]
    fn an_old_client_without_a_scope_is_answered_as_before() {
        let mut a = approvals();
        let p = a.request("agent-1", "run_command", "ls", 1, 0).unwrap();
        let r = resolve_msg(&p.id, true, "phone", "s3cret", "node-1");
        assert_eq!(r.scope, 0);
        assert_eq!(a.resolve(&r, 2).unwrap().1, Verdict::Approved);
        assert!(p.offer.scopes().is_empty(), "announced as before");
    }

    /// What a card offers, from the adapter's offer and the policy.
    #[test]
    fn the_card_offers_what_the_agent_offers_and_the_policy_allows() {
        use agora_protocol::v1::ApprovalScope as S;
        let card = |once: bool, always: bool| {
            Raise::Card(Offer {
                once,
                always: always.then(|| AlwaysOption {
                    label: "Always".into(),
                    option_id: "aa".into(),
                }),
            })
        };
        let both = [S::Once, S::Always];
        assert_eq!(
            raise_for(true, &both, "Always", "aa", true),
            card(true, true)
        );
        // An adapter that did not declare `approval_scopes` is offered the single answer, whatever it says.
        assert_eq!(
            raise_for(false, &both, "Always", "aa", true),
            card(true, false)
        );
        assert_eq!(
            raise_for(true, &[], "Always", "aa", true),
            card(true, false)
        );
        // The policy (or the manifest) removes the offer; an always-only request then gets no card.
        assert_eq!(
            raise_for(true, &both, "Always", "aa", false),
            card(true, false)
        );
        assert_eq!(
            raise_for(true, &[S::Always], "Always", "aa", true),
            card(false, true)
        );
        assert_eq!(
            raise_for(true, &[S::Always], "Always", "aa", false),
            Raise::AlwaysOnlyForbidden
        );
        // An option id that cannot be signed as it is is never offered.
        assert_eq!(
            raise_for(true, &both, "Always", "a\u{202e}a", true),
            card(true, false)
        );
        assert_eq!(
            raise_for(true, &[S::Always], "Always", "", true),
            Raise::Unanswerable
        );
        // The label is the agent's text: cleaned and bounded.
        let Raise::Card(o) = raise_for(
            true,
            &both,
            &format!("A\u{200b}{}", "x".repeat(300)),
            "aa",
            true,
        ) else {
            panic!("no card");
        };
        let label = o.always.unwrap().label;
        assert!(label.chars().count() <= MAX_ALWAYS_LABEL_CHARS && !label.contains('\u{200b}'));
        // What the node announces: empty for the single answer (as before ADR-0101), else the scopes in order.
        assert_eq!(offer(true, true).scopes(), vec![ONCE, ALWAYS]);
        assert_eq!(offer(false, true).scopes(), vec![ALWAYS]);
        assert!(Offer::once_only().scopes().is_empty());
    }

    /// The vectors every client signs by (`schema/fixtures/signing/approval-skip-v1.json`, also checked by agora-protocol).
    #[test]
    fn the_shared_skip_vectors_match() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/fixtures/signing/approval-skip-v1.json");
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for c in v["cases"].as_array().unwrap() {
            let s = |k: &str| c[k].as_str().unwrap().to_string();
            let binding = SkipBinding {
                agent: s("agent_id"),
                option: s("option_id"),
                value: s("value"),
            };
            let m = skip_signed_message(
                &s("node"),
                &s("approval_id"),
                c["approved"].as_bool().unwrap(),
                &binding,
                &s("detail"),
            );
            let hex: String = m.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hex, s("hex"), "{}", c["name"]);
        }
    }

    /// The detail the node writes is the one the vectors show: the signed agent, option and value are the detail's own.
    #[test]
    fn the_card_detail_names_what_is_signed() {
        let a = skip_ask("agent-1", SkipKind::Mode, "mode", "bypassPermissions");
        let d: serde_json::Value = serde_json::from_str(&a.detail("laptop")).unwrap();
        assert_eq!(d["agent"], a.binding().agent.as_str());
        assert_eq!(d["option"], a.binding().option.as_str());
        assert_eq!(d["value"], a.binding().value.as_str());
        assert_eq!(d["node"], "laptop");
        assert_eq!(d["folder"], "/work/login-fix");
        // The agent's own words are bounded and carry no control characters.
        let mut long = a.clone();
        long.value_name = format!("x\u{1b}[31m{}", "y".repeat(500));
        let d: serde_json::Value = serde_json::from_str(&long.detail("n")).unwrap();
        let shown = d["value_name"].as_str().unwrap();
        assert!(shown.chars().count() <= MAX_NAME_CHARS && !shown.contains('\u{1b}'));
    }

    /// A skip request is decided only over its own domain and the agent, option and value the node bound to it; the verdict is signed
    /// and a grant means nothing on it.
    #[test]
    fn a_skip_decision_is_verified_over_its_domain_and_binding() {
        let n = node_approvals();
        let ask = skip_ask("agent-1", SkipKind::Mode, "mode", "bypassPermissions");
        let binding = ask.binding();
        let raised = n.request_skip("node-1", ask, 1_000).unwrap();
        let p = raised.pending.clone();
        let wrong = |binding: &SkipBinding| skip_resolve(&n, &p, true, binding, 2_000);
        let other = |f: fn(&mut SkipBinding)| {
            let mut b = binding.clone();
            f(&mut b);
            b
        };
        assert_eq!(
            wrong(&other(|b| b.value = "default".into())).unwrap_err(),
            ApprovalError::BadSignature
        );
        assert_eq!(
            wrong(&other(|b| b.option = "auto".into())).unwrap_err(),
            ApprovalError::BadSignature
        );
        assert_eq!(
            wrong(&other(|b| b.agent = "agent-2".into())).unwrap_err(),
            ApprovalError::BadSignature
        );
        // Another request's message, the verdict flipped, an unknown device and a grant are refused too.
        let plain = ApprovalResolve {
            approval_id: p.id.clone(),
            approved: true,
            signer_device_id: "phone".into(),
            signature: sign("s3cret", &signed_message("node-1", &p.id, true)),
            helper_grant: 0,
            helpers_may_start: false,
            ..Default::default()
        };
        assert_eq!(
            n.resolve_request(&plain, 2_000).unwrap_err(),
            ApprovalError::BadSignature
        );
        let mut flipped = ApprovalResolve {
            signature: sign(
                "s3cret",
                &skip_signed_message("node-1", &p.id, false, &binding, &p.detail),
            ),
            ..plain.clone()
        };
        assert_eq!(
            n.resolve_request(&flipped, 2_000).unwrap_err(),
            ApprovalError::BadSignature
        );
        flipped.approved = false;
        flipped.signer_device_id = "stranger".into();
        assert_eq!(
            n.resolve_request(&flipped, 2_000).unwrap_err(),
            ApprovalError::BadSignature
        );
        let mut grant = ApprovalResolve {
            signature: sign(
                "s3cret",
                &skip_signed_message("node-1", &p.id, true, &binding, &p.detail),
            ),
            ..plain
        };
        grant.helper_grant = 2;
        assert_eq!(
            n.resolve_request(&grant, 2_000).unwrap_err(),
            ApprovalError::BadSignature
        );
        assert_eq!(n.pending_count(), 1, "nothing was consumed");
        // The right decision gives back exactly what the node recorded.
        let r = skip_resolve(&n, &p, true, &binding, 2_000).unwrap();
        assert_eq!(r.verdict, Verdict::Approved);
        let applied = r.skip.unwrap();
        assert_eq!(
            (applied.value.as_str(), applied.option_id.as_str()),
            ("bypassPermissions", "mode")
        );
        // Single use.
        assert_eq!(
            skip_resolve(&n, &p, true, &binding, 2_001).unwrap_err(),
            ApprovalError::Unknown
        );
    }

    /// A decision after the request's time is refused, applies nothing, and the sweep then forgets the record.
    #[test]
    fn a_late_skip_decision_is_refused() {
        let n = node_approvals();
        let ask = skip_ask("agent-1", SkipKind::Mode, "mode", "bypassPermissions");
        let binding = ask.binding();
        let p = n.request_skip("node-1", ask, 1_000).unwrap().pending;
        assert_eq!(p.expires_ms, 1_000 + SKIP_TTL_MS);
        let late = p.expires_ms;
        assert_eq!(
            skip_resolve(&n, &p, true, &binding, late).unwrap_err(),
            ApprovalError::Expired
        );
        assert_eq!(
            skip_resolve(&n, &p, true, &binding, late).unwrap_err(),
            ApprovalError::Unknown
        );
        // The sweep tells the node which request it was, and it has no adapter to answer.
        let swept = n.expire(late);
        assert_eq!(swept.len(), 1);
        assert!(n.forget_skip(&swept[0].0));
    }

    /// The node alone raises this request: an adapter that names this action gets nothing (its decision would apply a switch the agent
    /// asked for in text), and a plain request under the name can never be decided.
    #[test]
    fn an_adapter_cannot_raise_a_skip_request() {
        let n = node_approvals();
        let agent = agora_core::AgentId::new("agent-1");
        assert!(n
            .request(&agent, "ap-1", SKIP_ACTION, "{}", 1_000)
            .is_none());
        assert_eq!(n.pending_count(), 0);
        let mut a = approvals();
        let p = a.request("agent-1", SKIP_ACTION, "{}", 1_000, 0).unwrap();
        let r = ApprovalResolve {
            approval_id: p.id.clone(),
            approved: true,
            signer_device_id: "phone".into(),
            signature: sign("s3cret", &signed_message("node-1", &p.id, true)),
            helper_grant: 0,
            helpers_may_start: false,
            ..Default::default()
        };
        assert_eq!(a.resolve(&r, 2_000), Err(ApprovalError::BadSignature));
    }

    /// One card per agent, option and value; another value replaces it; another option or agent is its own card.
    #[test]
    fn skip_requests_are_deduplicated_and_replaced() {
        let n = node_approvals();
        let ask = |a: &str, v: &str| skip_ask(a, SkipKind::Mode, "mode", v);
        let first = n
            .request_skip("node-1", ask("agent-1", "bypass"), 1_000)
            .unwrap();
        assert!(first.fresh);
        let again = n
            .request_skip("node-1", ask("agent-1", "bypass"), 1_001)
            .unwrap();
        assert!(!again.fresh && again.pending.id == first.pending.id);
        assert_eq!(n.pending_count(), 1);
        let replaced = n
            .request_skip("node-1", ask("agent-1", "turbo"), 1_002)
            .unwrap();
        assert!(replaced.fresh && replaced.superseded == [first.pending.id.clone()]);
        assert_eq!(n.pending_count(), 1);
        let binding = skip_ask("agent-1", SkipKind::Mode, "mode", "bypass").binding();
        assert_eq!(
            skip_resolve(&n, &first.pending, true, &binding, 2_000).unwrap_err(),
            ApprovalError::Unknown,
            "the replaced card is spent"
        );
        // Another agent, and another kind of option with the same id, are cards of their own.
        n.request_skip("node-1", ask("agent-2", "turbo"), 1_003)
            .unwrap();
        n.request_skip(
            "node-1",
            skip_ask("agent-1", SkipKind::Config, "mode", "turbo"),
            1_004,
        )
        .unwrap();
        assert_eq!(n.pending_count(), 3);
    }

    /// Withdrawing: by agent, and by option; the turn's cancel leaves a card alone.
    #[test]
    fn skip_requests_are_withdrawn_with_their_agent_or_option() {
        let n = node_approvals();
        let agent = agora_core::AgentId::new("agent-1");
        let a = n
            .request_skip(
                "node-1",
                skip_ask("agent-1", SkipKind::Mode, "mode", "bypass"),
                1_000,
            )
            .unwrap()
            .pending;
        let b = n
            .request_skip(
                "node-1",
                skip_ask("agent-1", SkipKind::Config, "auto", "true"),
                1_000,
            )
            .unwrap()
            .pending;
        // A cancelled turn withdraws what its adapter asked, not a switch the user asked for.
        assert!(n.cancel_adapter_requests(&agent).is_empty());
        assert_eq!(n.pending_count(), 2);
        assert_eq!(
            n.withdraw_skips(&agent, Some((SkipKind::Mode, "mode"))),
            std::slice::from_ref(&a.id)
        );
        assert_eq!(n.pending_count(), 1);
        // Not this agent's, not that option: nothing.
        assert!(n
            .withdraw_skips(&agora_core::AgentId::new("agent-2"), None)
            .is_empty());
        assert!(n
            .withdraw_skips(&agent, Some((SkipKind::Mode, "mode")))
            .is_empty());
        assert_eq!(n.withdraw_skips(&agent, None), std::slice::from_ref(&b.id));
        assert_eq!(n.pending_count(), 0);
        // A withdrawn card is spent: its decision is nothing.
        let binding = skip_ask("agent-1", SkipKind::Mode, "mode", "bypass").binding();
        assert_eq!(
            skip_resolve(&n, &a, true, &binding, 2_000).unwrap_err(),
            ApprovalError::Unknown
        );
    }

    /// At most eight wait, and at most six are raised a minute; a repeat is no new request and a replacement is not counted twice.
    #[test]
    fn skip_requests_are_bounded() {
        let n = node_approvals();
        // The count: eight waiting, spread over a long time so the rate does not matter.
        for i in 0..MAX_SKIP_PENDING {
            let t = 1_000 + i as u64 * SKIP_WINDOW_MS;
            n.request_skip(
                "node-1",
                skip_ask(&format!("agent-{i}"), SkipKind::Mode, "mode", "bypass"),
                t,
            )
            .unwrap();
        }
        let t = 1_000 + MAX_SKIP_PENDING as u64 * SKIP_WINDOW_MS;
        assert_eq!(
            n.request_skip(
                "node-1",
                skip_ask("agent-x", SkipKind::Mode, "mode", "bypass"),
                t
            )
            .unwrap_err(),
            SkipRefusal::TooManyWaiting
        );
        // A repeat is still answered (it is the card that waits); a replacement counts as a new card and waits for room too.
        assert!(
            !n.request_skip(
                "node-1",
                skip_ask("agent-0", SkipKind::Mode, "mode", "bypass"),
                t
            )
            .unwrap()
            .fresh
        );
        assert_eq!(
            n.request_skip(
                "node-1",
                skip_ask("agent-0", SkipKind::Mode, "mode", "turbo"),
                t
            )
            .unwrap_err(),
            SkipRefusal::TooManyWaiting
        );
        // The rate: six in a minute, the seventh waits for the window.
        let n = node_approvals();
        for i in 0..MAX_SKIP_PER_WINDOW {
            n.request_skip(
                "node-1",
                skip_ask(&format!("agent-{i}"), SkipKind::Mode, "mode", "bypass"),
                5_000 + i as u64,
            )
            .unwrap();
        }
        assert_eq!(
            n.request_skip(
                "node-1",
                skip_ask("agent-7", SkipKind::Mode, "mode", "bypass"),
                6_000
            )
            .unwrap_err(),
            SkipRefusal::TooOften
        );
        assert!(n
            .request_skip(
                "node-1",
                skip_ask("agent-7", SkipKind::Mode, "mode", "bypass"),
                5_000 + SKIP_WINDOW_MS
            )
            .is_ok());
    }

    /// Adapters raise their own requests (`run_command`); every name in the node's own `agent.` namespace is refused, `agent.start` too:
    /// no card, nothing consumed, so the caller answers the adapter at once.
    #[test]
    fn an_adapter_cannot_use_any_name_of_the_nodes_own() {
        let n = node_approvals();
        let agent = agora_core::AgentId::new("agent-1");
        for action in [
            START_ACTION,
            SKIP_ACTION,
            "agent.stop",
            "agent.anything",
            "agent.",
        ] {
            assert!(
                n.request(&agent, "ap-1", action, "{}", 1_000).is_none(),
                "{action}"
            );
        }
        assert_eq!(n.pending_count(), 0);
        // Its own requests are cards as before.
        for action in ["run_command", "write_file", "agentic", "Agent.start"] {
            assert!(
                n.request(&agent, "ap-1", action, "ls", 1_000).is_some(),
                "{action}"
            );
        }
    }

    /// What an agent or a path can put in a card: hidden characters (controls, bidirectional and zero-width format characters, line
    /// separators) never reach the detail, long text is cut and marked; an id or value that cannot be signed as shown gets no card.
    #[test]
    fn a_card_shows_no_hidden_characters_and_is_bounded() {
        let hidden = "\u{202e}\u{2066}\u{200b}\u{200f}\u{2028}\u{feff}\u{1b}\u{85}";
        let mut a = skip_ask("agent-1", SkipKind::Mode, "mode", "bypass");
        a.agent_kind = format!("cl{hidden}aude{}", "k".repeat(200));
        a.folder = format!("/work/{hidden}login{}", "f".repeat(1000));
        a.option_name = format!("{hidden}Mode");
        a.value_name = format!("Safe{hidden} mode{}", "n".repeat(300));
        let detail = a.detail("laptop");
        let d: serde_json::Value = serde_json::from_str(&detail).unwrap();
        for k in ["agent_kind", "folder", "option_name", "value_name"] {
            let shown = d[k].as_str().unwrap();
            assert!(!shown.chars().any(is_hidden), "{k}: {shown:?}");
        }
        assert_eq!(d["option_name"], "Mode");
        assert!(d["agent_kind"].as_str().unwrap().starts_with("claude"));
        assert_eq!(
            d["agent_kind"].as_str().unwrap().chars().count(),
            MAX_KIND_CHARS
        );
        assert!(d["agent_kind"].as_str().unwrap().ends_with('\u{2026}'));
        assert_eq!(
            d["folder"].as_str().unwrap().chars().count(),
            MAX_FOLDER_CHARS
        );
        assert_eq!(
            d["value_name"].as_str().unwrap().chars().count(),
            MAX_NAME_CHARS
        );
        assert!(d["value_name"].as_str().unwrap().starts_with("Safe mode"));
        // The node's own judgement is in the card, apart from the agent's names.
        assert_eq!(d["skips"], true);
        // An option id or a value that would have to be altered to be shown, or is too long or empty, is never signed.
        let n = node_approvals();
        let refused = |kind: SkipKind, option: &str, value: &str| {
            n.request_skip("node-1", skip_ask("agent-1", kind, option, value), 1_000)
                .unwrap_err()
        };
        assert_eq!(
            refused(SkipKind::Mode, "mode", "by\u{202e}pass"),
            SkipRefusal::Unsignable
        );
        assert_eq!(
            refused(SkipKind::Mode, "mode", "by\u{200b}pass"),
            SkipRefusal::Unsignable
        );
        assert_eq!(
            refused(SkipKind::Mode, "mode", "bypass\n"),
            SkipRefusal::Unsignable
        );
        assert_eq!(refused(SkipKind::Mode, "mode", ""), SkipRefusal::Unsignable);
        assert_eq!(
            refused(SkipKind::Mode, "mode", &"v".repeat(MAX_SKIP_ID_BYTES + 1)),
            SkipRefusal::Unsignable
        );
        assert_eq!(
            refused(SkipKind::Config, &"o".repeat(MAX_SKIP_ID_BYTES + 1), "true"),
            SkipRefusal::Unsignable
        );
        assert_eq!(
            refused(SkipKind::Config, "a\u{2029}b", "true"),
            SkipRefusal::Unsignable
        );
        assert_eq!(n.pending_count(), 0, "nothing was raised");
        // The bound itself is fine.
        assert!(n
            .request_skip(
                "node-1",
                skip_ask(
                    "agent-1",
                    SkipKind::Mode,
                    "mode",
                    &"v".repeat(MAX_SKIP_ID_BYTES)
                ),
                1_000
            )
            .is_ok());
    }

    /// One agent's flood (waiting cards, cards per minute) is stopped by its own limits and never keeps another agent's card from being
    /// raised.
    #[test]
    fn one_agents_flood_does_not_block_another_agents_card() {
        let n = node_approvals();
        let ask = |a: &str, k: SkipKind, o: &str, v: &str| skip_ask(a, k, o, v);
        // Two waiting at most per agent.
        n.request_skip("node-1", ask("agent-1", SkipKind::Mode, "mode", "a"), 1_000)
            .unwrap();
        n.request_skip(
            "node-1",
            ask("agent-1", SkipKind::Config, "x", "true"),
            1_001,
        )
        .unwrap();
        assert_eq!(
            n.request_skip(
                "node-1",
                ask("agent-1", SkipKind::Config, "y", "true"),
                1_002
            )
            .unwrap_err(),
            SkipRefusal::AgentTooMany
        );
        // A replacement of one of them is a new card and waits for room too.
        assert_eq!(
            n.request_skip("node-1", ask("agent-1", SkipKind::Mode, "mode", "b"), 1_003)
                .unwrap_err(),
            SkipRefusal::AgentTooMany
        );
        // Another agent is not affected.
        assert!(
            n.request_skip("node-1", ask("agent-2", SkipKind::Mode, "mode", "a"), 1_004)
                .unwrap()
                .fresh
        );
        // Three raised a minute at most per agent: agent-3 raises and withdraws three times, the fourth in the window is refused.
        let agent = agora_core::AgentId::new("agent-3");
        for i in 0..MAX_SKIP_PER_AGENT_WINDOW {
            n.request_skip(
                "node-1",
                ask("agent-3", SkipKind::Mode, "mode", &format!("v{i}")),
                200_000 + i as u64,
            )
            .unwrap();
            n.withdraw_skips(&agent, None);
        }
        assert_eq!(
            n.request_skip(
                "node-1",
                ask("agent-3", SkipKind::Mode, "mode", "v9"),
                200_100
            )
            .unwrap_err(),
            SkipRefusal::AgentTooOften
        );
        // Still not blocking another agent, nor agent-3 once its window passed.
        assert!(n
            .request_skip(
                "node-1",
                ask("agent-4", SkipKind::Mode, "mode", "a"),
                200_100
            )
            .is_ok());
        assert!(n
            .request_skip(
                "node-1",
                ask("agent-3", SkipKind::Mode, "mode", "v9"),
                200_000 + SKIP_WINDOW_MS
            )
            .is_ok());
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
    fn a_start_decision_binds_the_request_and_the_grant() {
        let mut a = approvals();
        let detail = r#"{"kind":"codex","prompt":"fix the tests"}"#;
        let p = a.request("agent-1", START_ACTION, detail, 0, 0).unwrap();
        let mut r = resolve_msg(&p.id, true, "phone", "s3cret", "node-1");
        // The plain approval message does not decide a start.
        assert_eq!(a.resolve(&r, 1), Err(ApprovalError::BadSignature));
        // A decision over another request (a swapped prompt), or with another grant, does not either.
        let swapped = start_signed_message("node-1", &p.id, true, 0, false, "{}");
        r.signature = sign("s3cret", &swapped);
        assert_eq!(a.resolve(&r, 1), Err(ApprovalError::BadSignature));
        r.signature = sign(
            "s3cret",
            &start_signed_message("node-1", &p.id, true, 2, false, detail),
        );
        assert_eq!(a.resolve(&r, 1), Err(ApprovalError::BadSignature));
        r.helper_grant = 2;
        r.helpers_may_start = true;
        assert_eq!(a.resolve(&r, 1), Err(ApprovalError::BadSignature));
        r.helpers_may_start = false;
        assert_eq!(a.resolve(&r, 1).unwrap().1, Verdict::Approved);
    }

    /// The vector the web client's and the TUI's tests use too: every signer hashes the announced detail byte for byte (UTF-8,
    /// non-ASCII, a raw line break and an emoji included).
    pub const START_VECTOR_DETAIL: &str = "{\"prompt\":\"Grüße\nçalış 🚀\"}";
    pub const START_VECTOR_HASH: &str =
        "2705e6e894314e40e3938d6e7780386d393351c5d33a5826ed13948b56d8c076";

    #[test]
    fn the_start_message_hashes_the_detail_byte_for_byte() {
        let m = start_signed_message("node-1", "ap-1", true, 3, false, START_VECTOR_DETAIL);
        let want = format!("agora-approval-start-v1\0node-1\0ap-1\01\03\00\0{START_VECTOR_HASH}");
        assert_eq!(m, want.as_bytes());
    }

    /// The vectors the web client's test uses too (`schema/fixtures/signing/approval-start-v2.json`).
    #[test]
    fn the_shared_start_v2_vectors_match() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/fixtures/signing/approval-start-v2.json");
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for c in v["cases"].as_array().unwrap() {
            let detail = c["detail"].as_str().unwrap();
            // The settings are read out of the detail the way the node reads them.
            let settings = detail_settings(detail);
            let listed: Vec<(String, String)> = c["settings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    (
                        p[0].as_str().unwrap().to_string(),
                        p[1].as_str().unwrap().to_string(),
                    )
                })
                .collect();
            assert_eq!(settings, listed, "{}", c["name"]);
            let m = start_signed_message_v2(
                c["node"].as_str().unwrap(),
                c["approval_id"].as_str().unwrap(),
                c["approved"].as_bool().unwrap(),
                u32::try_from(c["helper_grant"].as_u64().unwrap()).unwrap(),
                c["helpers_may_start"].as_bool().unwrap(),
                &settings,
                detail,
            );
            let hex: String = m.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hex, c["hex"].as_str().unwrap(), "{}", c["name"]);
        }
    }

    /// The v3 vectors the web client's and the terminal client's tests use too (`schema/fixtures/signing/approval-start-v3.json`).
    #[test]
    fn the_shared_start_v3_vectors_match() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/fixtures/signing/approval-start-v3.json");
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for c in v["cases"].as_array().unwrap() {
            let detail = c["detail"].as_str().unwrap();
            // The settings and the options are read out of the detail the way the node reads them.
            let settings = detail_settings(detail);
            let options = detail_launch_options(detail);
            let listed: Vec<(String, String)> = c["settings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    (
                        p[0].as_str().unwrap().to_string(),
                        p[1].as_str().unwrap().to_string(),
                    )
                })
                .collect();
            let listed_options: Vec<String> = c["options"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| o.as_str().unwrap().to_string())
                .collect();
            assert_eq!(settings, listed, "{}", c["name"]);
            assert_eq!(options, listed_options, "{}", c["name"]);
            let m = start_signed_message_v3(
                c["node"].as_str().unwrap(),
                c["approval_id"].as_str().unwrap(),
                c["approved"].as_bool().unwrap(),
                u32::try_from(c["helper_grant"].as_u64().unwrap()).unwrap(),
                c["helpers_may_start"].as_bool().unwrap(),
                &settings,
                &options,
                detail,
            );
            let hex: String = m.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hex, c["hex"].as_str().unwrap(), "{}", c["name"]);
        }
    }

    /// ADR-0059 decision 5: a start whose detail names launch options is decided only over `agora-approval-start-v3`; the v1 and v2
    /// messages (what a client that does not know options signs) do not decide it, nor v3 without the options or with others.
    #[test]
    fn a_start_with_launch_options_is_decided_only_over_the_options_it_shows() {
        let shown = r#"{"kind":"codex","prompt":"fix","launch_options":[{"id":"skip-permissions","label":"option-skip-permissions","skips_permissions":true}]}"#;
        let options = detail_launch_options(shown);
        assert_eq!(options, ["skip-permissions"]);
        let decide = |over: &dyn Fn(&str) -> Vec<u8>| {
            let mut a = approvals();
            let p = a.request("agent-1", START_ACTION, shown, 0, 0).unwrap();
            let mut r = resolve_msg(&p.id, true, "phone", "s3cret", "node-1");
            r.signature = sign("s3cret", &over(&p.id));
            a.resolve(&r, 1).map(|(_, v)| v)
        };
        let v3 = |options: Vec<String>| {
            move |id: &str| {
                start_signed_message_v3("node-1", id, true, 0, false, &[], &options, shown)
            }
        };
        assert_eq!(decide(&v3(options.clone())), Ok(Verdict::Approved));
        for wrong in [
            Vec::new(),
            vec![
                "skip-permissions".to_string(),
                "skip-permissions".to_string(),
            ],
            vec!["plan".to_string()],
        ] {
            assert_eq!(decide(&v3(wrong)), Err(ApprovalError::BadSignature));
        }
        assert_eq!(
            decide(&|id| start_signed_message("node-1", id, true, 0, false, shown)),
            Err(ApprovalError::BadSignature)
        );
        assert_eq!(
            decide(&|id| start_signed_message_v2("node-1", id, true, 0, false, &[], shown)),
            Err(ApprovalError::BadSignature)
        );
    }

    /// ADR-0066: a start whose detail carries settings is decided only over `agora-approval-start-v2`; the v1 message (what a client
    /// that does not know settings signs, and what an old approval without them was) does not decide it, whether the settings were
    /// changed, added, dropped or reordered, and another device's signature does not either.
    #[test]
    fn a_start_with_settings_is_decided_only_over_the_settings_it_shows() {
        let detail =
            |settings: &str| format!(r#"{{"kind":"codex","prompt":"fix","settings":{settings}}}"#);
        let shown = detail(
            r#"[{"id":"model","label":"setting-model","value":"gpt-6.1-sol"},{"id":"effort","label":"setting-effort","value":"high"}]"#,
        );
        let signed_pairs = detail_settings(&shown);
        assert_eq!(signed_pairs.len(), 2);
        // `over` makes the bytes a client signs, given the request's id (which is random, so known only once it is made).
        let decide = |over: &dyn Fn(&str) -> Vec<u8>, device: &str, secret: &str| {
            let mut a = approvals();
            let p = a.request("agent-1", START_ACTION, &shown, 0, 0).unwrap();
            let mut r = resolve_msg(&p.id, true, device, secret, "node-1");
            r.signature = sign(secret, &over(&p.id));
            r.signer_device_id = device.into();
            a.resolve(&r, 1).map(|(_, v)| v)
        };
        let v2 = |settings: &[(String, String)], detail: &str| {
            let (settings, detail) = (settings.to_vec(), detail.to_string());
            move |id: &str| {
                start_signed_message_v2("node-1", id, true, 0, false, &settings, &detail)
            }
        };
        assert_eq!(
            decide(&v2(&signed_pairs, &shown), "phone", "s3cret"),
            Ok(Verdict::Approved)
        );
        // A client that signs the v1 message (it does not name the settings) is refused.
        assert_eq!(
            decide(
                &|id| start_signed_message("node-1", id, true, 0, false, &shown),
                "phone",
                "s3cret"
            ),
            Err(ApprovalError::BadSignature)
        );
        // The same request signed as if it had no settings, or other ones, or the same ones reordered or extended.
        let mut reordered = signed_pairs.clone();
        reordered.reverse();
        let mut changed = signed_pairs.clone();
        changed[0].1 = "gpt-6.1-terra".into();
        let mut extra = signed_pairs.clone();
        extra.push(("mode".into(), "plan".into()));
        for pairs in [
            Vec::new(),
            reordered,
            changed,
            extra,
            signed_pairs[..1].to_vec(),
        ] {
            assert_eq!(
                decide(&v2(&pairs, &shown), "phone", "s3cret"),
                Err(ApprovalError::BadSignature),
                "{pairs:?}"
            );
        }
        // A signature over another detail (the prompt or a setting swapped on the way) does not decide this one.
        let other = shown.replace("gpt-6.1-sol", "gpt-6.1-terra");
        assert_eq!(
            decide(&v2(&signed_pairs, &other), "phone", "s3cret"),
            Err(ApprovalError::BadSignature)
        );
        // Right bytes, wrong device (no such trusted key).
        assert_eq!(
            decide(&v2(&signed_pairs, &shown), "phone", "another-secret"),
            Err(ApprovalError::BadSignature)
        );
        // A request without settings still verifies over v1, and v2 with no settings listed is not what it is signed with.
        let plain = r#"{"kind":"codex","prompt":"fix"}"#;
        let mut a = approvals();
        let p = a.request("agent-1", START_ACTION, plain, 0, 0).unwrap();
        let mut r = resolve_msg(&p.id, true, "phone", "s3cret", "node-1");
        r.signature = sign(
            "s3cret",
            &start_signed_message_v2("node-1", &p.id, true, 0, false, &[], plain),
        );
        assert_eq!(a.resolve(&r, 1), Err(ApprovalError::BadSignature));
        r.signature = sign(
            "s3cret",
            &start_signed_message("node-1", &p.id, true, 0, false, plain),
        );
        assert_eq!(a.resolve(&r, 1).unwrap().1, Verdict::Approved);
    }

    #[test]
    fn a_grant_on_another_kind_of_request_is_refused() {
        let mut a = approvals();
        let p = a.request("agent-1", "write_file", "x", 0, 0).unwrap();
        let mut r = resolve_msg(&p.id, true, "phone", "s3cret", "node-1");
        r.helper_grant = 3;
        assert_eq!(a.resolve(&r, 1), Err(ApprovalError::BadSignature));
        assert_eq!(a.pending().count(), 1);
    }

    /// A helper grant of agent `a` for `codex` in `/w`, and the decision it came with signed by `phone`.
    fn helper_grant(id: &str, secret: &str) -> (HelperGrant, Vec<u8>, Vec<u8>) {
        let message = start_signed_message("node-1", id, true, 3, false, "{}");
        let signature = sign(secret, &message);
        let g = HelperGrant {
            id: id.into(),
            agent: agora_core::AgentId::new("a"),
            kind: "codex".into(),
            folder: "/w".into(),
            worktree: false,
            remaining: 2,
            helpers_may_start: false,
            signer_device_id: "phone".into(),
        };
        (g, message, signature)
    }

    fn add(n: &NodeApprovals, id: &str) {
        let (g, m, s) = helper_grant(id, "s3cret");
        n.add_helper_grant(g, m, s);
    }

    #[test]
    fn helper_grants_are_used_up_revoked_and_end_with_their_agent() {
        let n = NodeApprovals::new("node-1", Box::new(Fake(vec![("phone", "s3cret")])));
        let me = agora_core::AgentId::new("a");
        add(&n, "g1");
        assert_eq!(n.use_helper_grant(&me, "codex", "/other", false), None);
        assert_eq!(n.use_helper_grant(&me, "claude", "/w", false), None);
        assert_eq!(
            n.use_helper_grant(&agora_core::AgentId::new("b"), "codex", "/w", false),
            None
        );
        // A grant from a start in the folder does not cover a start in a worktree of it.
        assert_eq!(n.use_helper_grant(&me, "codex", "/w", true), None);
        assert_eq!(n.use_helper_grant(&me, "codex", "/w", false), Some(false));
        assert_eq!(n.use_helper_grant(&me, "codex", "/w", false), Some(false));
        assert_eq!(n.use_helper_grant(&me, "codex", "/w", false), None);
        add(&n, "g2");
        assert!(n.revoke_helper_grant("g2"));
        assert!(n.helper_grants().is_empty());
        add(&n, "g3");
        n.end_helper_grants(&me);
        assert!(n.helper_grants().is_empty());
        // A decision that does not verify (another device's secret) gives no grant.
        let (g, m, _) = helper_grant("g4", "s3cret");
        n.add_helper_grant(g, m, b"forged".to_vec());
        assert!(n.helper_grants().is_empty());
    }

    /// Trusts these devices, each in its term (signatures as [`sign`] makes them with the device's secret).
    struct Terms(Vec<(&'static str, &'static str, Term)>);

    impl Verifier for Terms {
        fn verify(&self, device: &str, message: &[u8], signature: &[u8]) -> bool {
            self.0
                .iter()
                .any(|(d, secret, _)| *d == device && sign(secret, message) == signature)
        }
        fn term(&self, device: &str) -> Option<Term> {
            self.0.iter().find(|(d, _, _)| *d == device).map(|x| x.2)
        }
    }

    /// Audit F290: a helper grant starts agents only while the device that signed it still vouches for it.
    #[test]
    fn a_helper_grant_ends_when_its_signer_is_no_longer_trusted() {
        let me = agora_core::AgentId::new("a");
        let first = Term {
            since: Some(1),
            until: None,
        };
        let phone = |term| Box::new(Terms(vec![("phone", "s3cret", term)]));
        let n = NodeApprovals::new("node-1", phone(first));
        add(&n, "g1");
        assert_eq!(n.helper_grants().len(), 1);
        // The device is removed: its grant covers nothing, ends, and does not come back when the device is trusted again.
        n.set_verifier(Box::new(Terms(vec![])));
        assert_eq!(n.use_helper_grant(&me, "codex", "/w", false), None);
        n.set_verifier(phone(first));
        assert_eq!(n.use_helper_grant(&me, "codex", "/w", false), None);
        assert!(n.helper_grants().is_empty());

        // Removed and trusted again unseen by this node (a new term): the old grant does not carry over.
        add(&n, "g2");
        n.set_verifier(phone(Term {
            since: Some(2),
            until: None,
        }));
        assert!(n.helper_grants().is_empty(), "a later term");

        // An endorsed device whose trust ran out.
        n.set_verifier(phone(Term {
            since: Some(2),
            until: Some(1),
        }));
        add(&n, "g3");
        assert_eq!(
            n.use_helper_grant(&me, "codex", "/w", false),
            None,
            "ran out"
        );

        // The device's limits narrowed to another project: the grant ends.
        n.set_verifier(phone(first));
        add(&n, "g4");
        assert_eq!(n.use_helper_grant(&me, "codex", "/w", false), Some(false));
        let limits = std::sync::Arc::new(crate::limits::DeviceLimits::new(
            [("phone".to_string(), ["other".to_string()].into())].into(),
        ));
        limits.set_project_resolver(Box::new(|_| Some("mine".to_string())));
        n.set_limits(limits);
        assert_eq!(
            n.use_helper_grant(&me, "codex", "/w", false),
            None,
            "narrowed"
        );
        assert!(n.helper_grants().is_empty());
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
                helper_grant: 0,
                helpers_may_start: false,
                ..Default::default()
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
            helper_grant: 0,
            helpers_may_start: false,
            ..Default::default()
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
            helper_grant: 0,
            helpers_may_start: false,
            ..Default::default()
        };
        assert_eq!(a.resolve(&r, 1).unwrap().1, Verdict::Approved);
    }

    /// Security review of PR #1190, S1: real signers sign with a random nonce; this stand-in accepts any one byte after the message.
    struct Nonced;

    impl Verifier for Nonced {
        fn verify(&self, device: &str, message: &[u8], signature: &[u8]) -> bool {
            device == "phone"
                && signature.len() == message.len() + 1
                && signature.starts_with(message)
        }
    }

    fn nonced(id: &str, approved: bool, nonce: u8) -> ApprovalResolve {
        let mut signature = signed_message("node-1", id, approved);
        signature.push(nonce);
        ApprovalResolve {
            approval_id: id.into(),
            approved,
            signer_device_id: "phone".into(),
            signature,
            ..Default::default()
        }
    }

    /// ADR-0110 decision 6, S1: a request put back after its answer did not reach the adapter takes a new answer, never the same
    /// signed one again (a hub replaying the person's earlier answer over a newer one).
    #[test]
    fn a_request_put_back_takes_a_new_answer_never_the_same_signed_one_again() {
        let n = NodeApprovals::new("node-1", Box::new(Nonced));
        let agent = agora_core::AgentId::new("agent-1");
        let p = n.request(&agent, "ap-1", "run_command", "ls", 1).unwrap();
        let first = nonced(&p.id, true, 1);
        let taken = n.resolve_request(&first, 2).unwrap();
        assert!(n.restore(&taken, 3));
        assert_eq!(
            n.resolve_request(&first, 4).unwrap_err(),
            ApprovalError::BadSignature
        );
        let second = n.resolve_request(&nonced(&p.id, false, 2), 5).unwrap();
        assert_eq!(second.verdict, Verdict::Denied);
        assert_eq!(second.adapter_approval_id, "ap-1");
    }

    /// `n - s` for a P-256 signature `r || s` (`n` the group order): the other signature of the same message, which verifies too.
    fn flip_s(signature: &[u8]) -> Vec<u8> {
        const N: [u8; 32] = [
            0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xbc, 0xe6, 0xfa, 0xad, 0xa7, 0x17, 0x9e, 0x84, 0xf3, 0xb9, 0xca, 0xc2,
            0xfc, 0x63, 0x25, 0x51,
        ];
        let mut out = signature[..32].to_vec();
        let mut s = [0u8; 32];
        let mut borrow = 0i16;
        for i in (0..32).rev() {
            let mut d = i16::from(N[i]) - i16::from(signature[32 + i]) - borrow;
            borrow = i16::from(d < 0);
            if d < 0 {
                d += 256;
            }
            s[i] = u8::try_from(d).unwrap();
        }
        out.extend_from_slice(&s);
        out
    }

    /// Re-review of PR #1190, R1: with a real P-256 key, a request put back refuses the same signed answer and the one whose `s` a
    /// relay replaced by `n - s` (which the key's check accepts), and takes a freshly signed answer.
    #[test]
    fn a_put_back_request_refuses_a_replayed_p256_answer_even_with_s_flipped() {
        use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
            .unwrap();
        let mut verifier = P256Verifier::new();
        assert!(verifier.trust("phone", key.public_key().as_ref()));
        let sign = |id: &str, approved: bool| {
            key.sign(&rng, &signed_message("node-1", id, approved))
                .unwrap()
                .as_ref()
                .to_vec()
        };
        let n = NodeApprovals::new("node-1", Box::new(verifier));
        let agent = agora_core::AgentId::new("agent-1");
        let p = n.request(&agent, "ap-1", "run_command", "ls", 1).unwrap();
        let answer = |signature: Vec<u8>, approved: bool| ApprovalResolve {
            approval_id: p.id.clone(),
            approved,
            signer_device_id: "phone".into(),
            signature,
            ..Default::default()
        };
        let first = sign(&p.id, true);
        let flipped = flip_s(&first);
        assert_ne!(flipped, first);
        let taken = n.resolve_request(&answer(first.clone(), true), 2).unwrap();
        assert!(n.restore(&taken, 3));
        // The flipped signature is one the key's check accepts: only the replay rule stands in its way.
        assert!(n.verify_device("phone", &signed_message("node-1", &p.id, true), &flipped));
        for replay in [first, flipped] {
            assert_eq!(
                n.resolve_request(&answer(replay, true), 4).unwrap_err(),
                ApprovalError::BadSignature
            );
        }
        let fresh = n
            .resolve_request(&answer(sign(&p.id, true), true), 5)
            .unwrap();
        assert_eq!(fresh.verdict, Verdict::Approved);
    }

    /// R2: a request handed to the sweep as expired while its answer call ran, whose answer then came late, is recorded once: by the
    /// call. The sweep neither sees it nor, if it already took it, records it again.
    #[test]
    fn a_late_answer_to_a_request_handed_to_the_sweep_is_the_only_record() {
        let n = NodeApprovals::new("node-1", Box::new(Nonced));
        let agent = agora_core::AgentId::new("agent-1");
        for swept_first in [false, true] {
            let p = n.request(&agent, "ap-1", "run_command", "ls", 1).unwrap();
            let taken = n.resolve_request(&nonced(&p.id, true, 1), 2).unwrap();
            assert!(n.flight_begin(&p.id));
            n.flight_watched(&p.id);
            assert_eq!(
                n.restore_unless_recorded(&taken, 2 + DEFAULT_TTL_MS, false),
                Some(false)
            );
            if swept_first {
                let swept: Vec<String> = n
                    .expire(3 + DEFAULT_TTL_MS)
                    .into_iter()
                    .map(|x| x.0)
                    .collect();
                assert_eq!(swept, std::slice::from_ref(&p.id));
                assert!(n.flight_delivered(&p.id, true));
                n.take_pending(&p.id);
                assert!(!n.flight_ended(&p.id), "recorded by the call already");
            } else {
                assert!(n.flight_delivered(&p.id, true));
                n.take_pending(&p.id);
                assert!(
                    n.expire(3 + DEFAULT_TTL_MS).is_empty(),
                    "the sweep no longer ends it"
                );
            }
        }
    }

    /// R3: a call that ends without its answer reaching the agent never puts back a request the followed call recorded meanwhile.
    #[test]
    fn a_request_recorded_meanwhile_is_never_put_back() {
        let n = NodeApprovals::new("node-1", Box::new(Nonced));
        let agent = agora_core::AgentId::new("agent-1");
        let p = n.request(&agent, "ap-1", "run_command", "ls", 1).unwrap();
        let first = n.resolve_request(&nonced(&p.id, true, 1), 2).unwrap();
        assert!(n.flight_begin(&p.id));
        n.flight_watched(&p.id);
        assert_eq!(n.restore_unless_recorded(&first, 3, false), Some(true));
        // A second answer takes the request; its call is not followed (one per request).
        let second = n.resolve_request(&nonced(&p.id, false, 2), 4).unwrap();
        assert!(!n.flight_begin(&p.id));
        // The followed call records the first answer before the second call puts the request back.
        assert!(n.flight_delivered(&p.id, true));
        assert_eq!(n.restore_unless_recorded(&second, 5, true), None);
        assert_eq!(n.pending_count(), 0);
        assert!(n.expire(1 + DEFAULT_TTL_MS).is_empty());
        assert!(!n.flight_ended(&p.id));
    }

    /// S2: a request is recorded once, with the first answer that reached the agent; "cancelled" only when no call that may still
    /// deliver an answer runs.
    #[test]
    fn a_request_is_recorded_once_with_the_first_answer_that_reached_the_agent() {
        let n = NodeApprovals::new("node-1", Box::new(Nonced));
        // (a) A followed call and a second answer the adapter refused, in either order: the followed call's answer is recorded.
        for refused_first in [true, false] {
            assert!(n.flight_begin("x"));
            n.flight_watched("x");
            assert!(!n.flight_begin("x"), "one followed call per request (N1)");
            if refused_first {
                assert!(
                    !n.flight_undelivered("x", true, false),
                    "waits for the followed call"
                );
                assert!(n.flight_delivered("x", true));
            } else {
                assert!(n.flight_delivered("x", true));
                assert!(!n.flight_undelivered("x", true, false));
            }
            assert!(n.fl().map.is_empty(), "nothing kept once both ended");
        }
        // The followed call ends without an answer after the refusal: recorded as cancelled then, once.
        n.flight_begin("y");
        n.flight_watched("y");
        n.flight_begin("y");
        assert!(!n.flight_undelivered("y", true, false));
        assert!(n.flight_undelivered("y", false, true));
        // A followed call that runs out owes nothing by itself: the card stays.
        n.flight_begin("z");
        n.flight_watched("z");
        assert!(!n.flight_undelivered("z", false, true));
        assert!(n.fl().map.is_empty());
    }

    /// S2 (b), N5: a kept request that expires while its answer call is still followed is recorded by that call (with the answer when
    /// it reached the agent), never by the sweep as well; one with no call is recorded by the sweep.
    #[test]
    fn a_kept_request_that_expires_while_its_answer_call_runs_is_recorded_by_that_call() {
        let n = NodeApprovals::new("node-1", Box::new(Nonced));
        let agent = agora_core::AgentId::new("agent-1");
        for delivered in [true, false] {
            let p = n.request(&agent, "ap-1", "run_command", "ls", 1).unwrap();
            let taken = n.resolve_request(&nonced(&p.id, true, 1), 2).unwrap();
            assert!(n.flight_begin(&p.id));
            n.flight_watched(&p.id);
            assert!(n.restore(&taken, 3));
            let swept: Vec<String> = n
                .expire(1 + DEFAULT_TTL_MS)
                .into_iter()
                .map(|x| x.0)
                .collect();
            assert_eq!(swept, std::slice::from_ref(&p.id));
            assert!(
                !n.flight_ended(&p.id),
                "the call still runs: the sweep records nothing"
            );
            if delivered {
                assert!(
                    n.flight_delivered(&p.id, true),
                    "the answer reached the agent"
                );
            } else {
                assert!(n.flight_undelivered(&p.id, false, true), "cancelled, once");
            }
        }
        // No call: the sweep records it.
        assert!(n.flight_ended("never-answered"));
    }

    /// S2: a request its agent's stop (or a cancelled turn) withdrew is recorded by that stop: a followed call ending later records
    /// nothing, even with an answer.
    #[test]
    fn a_kept_request_withdrawn_by_the_agents_stop_is_not_recorded_again() {
        let n = NodeApprovals::new("node-1", Box::new(Nonced));
        let agent = agora_core::AgentId::new("agent-1");
        let p = n.request(&agent, "ap-1", "run_command", "ls", 1).unwrap();
        let taken = n.resolve_request(&nonced(&p.id, true, 1), 2).unwrap();
        n.flight_begin(&p.id);
        n.flight_watched(&p.id);
        assert!(n.restore(&taken, 3));
        assert_eq!(n.cancel_agent(&agent), std::slice::from_ref(&p.id));
        assert!(!n.flight_delivered(&p.id, true));
    }

    /// B1 and the restore of a request that ran out meanwhile: it is not put back but handed to the next sweep, once.
    #[test]
    fn a_request_that_expired_during_the_wait_is_handed_to_the_sweep() {
        let n = NodeApprovals::new("node-1", Box::new(Nonced));
        let agent = agora_core::AgentId::new("agent-1");
        let p = n.request(&agent, "ap-1", "run_command", "ls", 1).unwrap();
        let taken = n.resolve_request(&nonced(&p.id, true, 1), 2).unwrap();
        assert!(!n.restore(&taken, 2 + DEFAULT_TTL_MS));
        assert_eq!(n.pending_count(), 0);
        let swept: Vec<String> = n
            .expire(3 + DEFAULT_TTL_MS)
            .into_iter()
            .map(|x| x.0)
            .collect();
        assert_eq!(swept, [p.id]);
        assert!(n.expire(4 + DEFAULT_TTL_MS).is_empty());
    }

    #[test]
    fn a_decision_that_arrives_after_expiry_still_ends_with_the_agent_told_denied() {
        let n = NodeApprovals::new("node-1", Box::new(Fake(vec![("phone", "s3cret")])));
        let agent = agora_core::AgentId::new("agent-1");
        let p = n.request(&agent, "ap-1", "run_command", "ls", 1).unwrap();
        // Signed and valid, but at the expiry instant, before the node's sweep ran: refused, and replays stay refused.
        let late = resolve_msg(&p.id, true, "phone", "s3cret", "node-1");
        assert_eq!(
            n.resolve(&late, 1 + DEFAULT_TTL_MS).unwrap_err(),
            ApprovalError::Expired
        );
        assert_eq!(
            n.resolve(&late, 2 + DEFAULT_TTL_MS).unwrap_err(),
            ApprovalError::Unknown
        );
        // The sweep still tells the adapter, once.
        let swept: Vec<(String, String, String)> = n
            .expire(2 + DEFAULT_TTL_MS)
            .into_iter()
            .map(|(id, a, ad)| (id, a.as_str().to_string(), ad))
            .collect();
        assert_eq!(swept, [(p.id.clone(), "agent-1".into(), "ap-1".into())]);
        assert!(n.expire(3 + DEFAULT_TTL_MS).is_empty());
        // Unless the agent stopped in between: nothing is sent to it.
        let p = n.request(&agent, "ap-2", "run_command", "ls", 10).unwrap();
        let late = resolve_msg(&p.id, true, "phone", "s3cret", "node-1");
        assert!(n.resolve(&late, 10 + DEFAULT_TTL_MS).is_err());
        n.cancel_agent(&agent);
        assert!(n.expire(10 + DEFAULT_TTL_MS).is_empty());
    }

    #[test]
    fn a_device_limited_to_other_projects_cannot_answer_an_agents_request() {
        let n = NodeApprovals::new(
            "node-1",
            Box::new(Fake(vec![("phone", "s3cret"), ("eda", "s3cret")])),
        );
        let limits = std::sync::Arc::new(crate::limits::DeviceLimits::new(
            [("eda".to_string(), ["web".to_string()].into())].into(),
        ));
        limits.set_project_resolver(Box::new(|a| match a {
            "agent-web" => Some("web".into()),
            "agent-api" => Some("api".into()),
            _ => None,
        }));
        n.set_limits(limits);
        let api = n
            .request(
                &agora_core::AgentId::new("agent-api"),
                "ap-1",
                "run",
                "x",
                0,
            )
            .unwrap();
        let web = n
            .request(
                &agora_core::AgentId::new("agent-web"),
                "ap-2",
                "run",
                "x",
                0,
            )
            .unwrap();
        let msg = |id: &str, device: &str| resolve_msg(id, true, device, "s3cret", "node-1");
        // The limited device is refused for the other project, and nothing is consumed: the owner's phone can still answer it.
        assert_eq!(
            n.resolve(&msg(&api.id, "eda"), 1).unwrap_err(),
            ApprovalError::Forbidden
        );
        assert!(n.resolve(&msg(&web.id, "eda"), 1).is_ok());
        assert!(n.resolve(&msg(&api.id, "phone"), 1).is_ok());
    }

    /// Codex review of #1027: a turn cancelled between an adapter request entering the queue and its route being recorded (control and
    /// the daemon run at once) leaves that request alone. Withdrawn there, the daemon would still record an orphan route and announce
    /// a request that is gone. Only fully registered adapter requests are cancelled.
    #[test]
    fn a_turn_cancelled_before_a_request_is_routed_leaves_it_alone() {
        let n = node_approvals();
        let agent = agora_core::AgentId::new("agent-1");
        let p = n
            .raise_adapter(&agent, "run_command", "ls", 1_000, Offer::once_only())
            .unwrap();
        // The cancel runs between the two halves of the registration.
        assert!(n.cancel_adapter_requests(&agent).is_empty());
        n.route_adapter(&p, &agent, "ap-1");
        assert_eq!(n.agent_of(&p.id), Some(agent.clone()));
        let waiting: Vec<String> = n.waiting().into_iter().map(|w| w.id).collect();
        assert_eq!(waiting, vec![p.id.clone()]);
        // Once routed, the next cancelled turn withdraws it, route and all.
        assert_eq!(n.cancel_adapter_requests(&agent), vec![p.id.clone()]);
        assert!(n.waiting().is_empty());
        assert_eq!(n.agent_of(&p.id), None);
    }

    #[test]
    fn cancel_adapter_requests_leaves_agent_start_requests_waiting() {
        let n = node_approvals();
        let agent = agora_core::AgentId::new("agent-1");
        let start = n
            .request_start(&agent, r#"{"kind":"codex"}"#, 1_000)
            .unwrap();
        let adapter = n
            .request(&agent, "ap-1", "run_command", "ls", 1_000)
            .unwrap();
        assert_eq!(n.cancel_adapter_requests(&agent), vec![adapter.id]);
        let waiting: Vec<String> = n.waiting().into_iter().map(|p| p.id).collect();
        assert_eq!(waiting, vec![start.id]);
    }
}
