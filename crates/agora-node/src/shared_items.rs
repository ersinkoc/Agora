//! ADR-0122 step 3: the shared skills and MCP servers this node gives its agents. The node is the authority, the hub only the first
//! gate: every word the hub hands over is verified here against this node's own trusted devices (ADR-0061, a device limited to some
//! projects never counts), its content hash and the rules in `agora_protocol::share`, and the newest `issued_ms` this node ever saw for
//! the name (kept on disk, so a replayed older word is refused even after a restart); then this node's policy decides
//! (`share.items`, `share.mcp_commands`). The hub can only narrow: an item it stops listing is dropped.
//!
//! The hub is asked for the list (names and hashes, no content) when the link is set and every [`POLL`]; a changed hash is fetched
//! with `ShareItemGet`. Each refusal is said once per version with `ShareItemRefused`. What a launch gets is [`SharedItems::for_launch`]
//! (step 3b injects it).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agora_protocol::share::{
    check_item, check_scope, content_bytes, signed_message, valid_name, Action,
};
use agora_protocol::v1::{
    self, command_ok, command_request, event::Body, ShareItemInfo, ShareItemSigned,
};
use prost::Message;

use crate::approvals::NodeApprovals;

/// Whether a device this node trusts (in its term, not limited to projects) signed `message`: `(signer, message, signature, now_ms)`.
pub type Vouch = Box<dyn Fn(&str, &[u8], &[u8], u64) -> bool + Send + Sync>;

/// How often the hub is asked for the list.
pub const POLL: Duration = Duration::from_secs(60);
/// How soon the list is asked again when the hub could not answer (the link not up yet, or down).
pub const RETRY: Duration = Duration::from_secs(5);
/// The folder under the node's data folder.
pub const DIR: &str = "shared-items";
const NEWEST_FILE: &str = "newest";

/// This node's word on shared items (`policy`): `share.items = off` gives none; `share.mcp_commands`, when set, are the only
/// programs a shared MCP server may run here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharePolicy {
    pub on: bool,
    pub mcp_commands: Option<BTreeSet<String>>,
    /// Whether a shared MCP server at a URL may be given here (`share.mcp_urls = allow | forbid`, default allow). It runs nothing on
    /// this machine, but it is still a channel: an agent sends it repository content as tool arguments and reads its results.
    pub mcp_urls: bool,
}

impl Default for SharePolicy {
    fn default() -> Self {
        Self {
            on: true,
            mcp_commands: None,
            mcp_urls: true,
        }
    }
}

/// Why a word is not used here: a stable code and a reason word for `ShareItemRefused`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub code: &'static str,
    pub reason: &'static str,
}

fn refusal(code: &'static str, reason: &'static str) -> Refusal {
    Refusal { code, reason }
}

fn sha256_hex(b: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, b)
        .as_ref()
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

/// An item's content hash, as the signature names it.
pub fn item_hash(item: &v1::ShareItem) -> String {
    sha256_hex(&content_bytes(item, sha256_hex))
}

/// Whether `w` may be used here, judged with this node's own trust (`vouched`: the signer is a trusted device of this node, in its
/// term and not limited to projects), the newest `issued_ms` this node saw for the name, and its policy. A removal (no item) only has
/// to be vouched and newer.
pub fn judge(
    w: &ShareItemSigned,
    newest: Option<u64>,
    policy: &SharePolicy,
    vouched: impl FnOnce(&str, &[u8], &[u8]) -> bool,
) -> Result<(), Refusal> {
    let scope = w.scope.clone().unwrap_or_default();
    if !valid_name(&w.name, 64) || check_scope(&scope).is_err() {
        return Err(refusal("share.refused", "invalid"));
    }
    let (action, hash) = match &w.item {
        Some(item) => {
            if item.name != w.name || check_item(item).is_err() {
                return Err(refusal("share.refused", "invalid"));
            }
            let hash = item_hash(item);
            if hash != w.item_hash {
                return Err(refusal("share.refused", "invalid"));
            }
            (Action::Publish, hash)
        }
        None if w.item_hash.is_empty() => (Action::Remove, String::new()),
        None => return Err(refusal("share.refused", "invalid")),
    };
    let message = signed_message(action, &w.name, &hash, &scope, w.issued_ms);
    if !vouched(&w.signer_device_id, &message, &w.signature) {
        return Err(refusal("approval.signature_invalid", "signature"));
    }
    // The same word again (read back after a restart) is fine; one older than the newest seen never is.
    if newest.is_some_and(|n| n > w.issued_ms) {
        return Err(refusal("share.refused", "older"));
    }
    if action == Action::Publish {
        if !policy.on {
            return Err(refusal("share.refused", "policy_off"));
        }
        let program = w
            .item
            .as_ref()
            .and_then(|i| i.mcp.as_ref())
            .map(|m| m.command.as_str())
            .filter(|c| !c.is_empty());
        if let (Some(cmd), Some(allowed)) = (program, &policy.mcp_commands) {
            if !allowed.contains(cmd) {
                return Err(refusal("share.refused", "command_not_allowed"));
            }
        }
        let url = w
            .item
            .as_ref()
            .and_then(|i| i.mcp.as_ref())
            .is_some_and(|m| !m.url.is_empty());
        if url && !policy.mcp_urls {
            return Err(refusal("share.refused", "url_not_allowed"));
        }
    }
    Ok(())
}

/// Whether an item's scope covers an agent of `kind` in `project`.
pub fn covers(scope: &v1::ShareScope, project: &str, kind: &str) -> bool {
    (scope.project_id.is_empty() || scope.project_id == project)
        && (scope.kinds.is_empty() || scope.kinds.iter().any(|k| k == kind))
}

enum FromLink {
    Listed(Result<Vec<ShareItemInfo>, String>),
    Got(String, String, Result<Box<ShareItemSigned>, String>),
}

#[derive(Default)]
struct State {
    policy: SharePolicy,
    /// The words in use, by name (verified, kept on disk).
    used: BTreeMap<String, ShareItemSigned>,
    /// The newest `issued_ms` seen for each name (kept on disk; never goes back).
    newest: BTreeMap<String, u64>,
    /// `(name, hash)` refused already (said once each).
    refused: BTreeSet<(String, String)>,
    /// `(name, hash)` being fetched.
    asking: BTreeSet<(String, String)>,
    next_poll: Option<Instant>,
}

pub struct SharedItems {
    dir: PathBuf,
    node_id: String,
    vouch: Vouch,
    state: Mutex<State>,
    link: Mutex<Option<Arc<crate::uplink_client::MeshOut>>>,
    from_link: Arc<Mutex<Vec<FromLink>>>,
    /// Facts to record (`ShareItemRefused`), taken by the daemon's loop.
    out: Mutex<Vec<Body>>,
    /// ADR-0122 step 3b: what each running agent was given at its launch (`AgentStarted.shared_items`).
    given: Mutex<BTreeMap<String, Vec<v1::SharedItemRef>>>,
}

/// `{secret:NAME}` in `value` filled from `secret`; the name of the first secret that is missing, as the error.
pub fn fill_secrets(
    value: &str,
    secret: &dyn Fn(&str) -> Option<String>,
) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = value;
    while let Some(at) = rest.find("{secret:") {
        out.push_str(&rest[..at]);
        let after = &rest[at + "{secret:".len()..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[at..]);
            return Ok(out);
        };
        let name = &after[..end];
        let plain = !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        match secret(name).filter(|_| plain) {
            Some(v) => out.push_str(&v),
            None => return Err(name.chars().take(64).collect()),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

impl SharedItems {
    /// Opens the node's shared items under `data` (what was used before is checked again, with today's trust and policy).
    pub fn open(
        data: &Path,
        node_id: &str,
        approvals: Arc<NodeApprovals>,
        policy: SharePolicy,
    ) -> Self {
        Self::open_with(
            data,
            node_id,
            Box::new(move |signer, message, signature, now_ms| {
                approvals
                    .remote_grant_term(signer, message, signature, now_ms)
                    .is_some()
            }),
            policy,
        )
    }

    /// [`Self::open`] with the trust check given (tests).
    pub fn open_with(data: &Path, node_id: &str, vouch: Vouch, policy: SharePolicy) -> Self {
        let dir = data.join(DIR);
        let newest: BTreeMap<String, u64> = std::fs::read_to_string(dir.join(NEWEST_FILE))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let me = Self {
            dir,
            node_id: node_id.to_string(),
            vouch,
            state: Mutex::new(State {
                policy,
                newest,
                ..State::default()
            }),
            link: Mutex::new(None),
            from_link: Arc::new(Mutex::new(Vec::new())),
            out: Mutex::new(Vec::new()),
            given: Mutex::new(BTreeMap::new()),
        };
        let kept: Vec<ShareItemSigned> = std::fs::read_dir(&me.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "pb"))
            .filter_map(|e| std::fs::read(e.path()).ok())
            .filter_map(|b| ShareItemSigned::decode(b.as_slice()).ok())
            .collect();
        for w in kept {
            me.take(w);
        }
        me
    }

    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The hub link, set when it is up (the list is asked at the next tick).
    pub fn set_link(&self, link: Arc<crate::uplink_client::MeshOut>) {
        *self.link.lock().unwrap_or_else(|e| e.into_inner()) = Some(link);
        self.st().next_poll = None;
    }

    /// Whether this node's policy gives shared items.
    pub fn on(&self) -> bool {
        self.st().policy.on
    }

    pub fn set_policy(&self, policy: SharePolicy) {
        let mut st = self.st();
        if !policy.on {
            st.used.clear();
        }
        st.policy = policy;
        st.refused.clear();
        st.next_poll = None;
    }

    /// The words an agent of `kind` in `project` gets at its start, by name, at most `agora_protocol::share::MAX_PER_AGENT`.
    pub fn for_launch(&self, project: &str, kind: &str) -> Vec<ShareItemSigned> {
        let st = self.st();
        if !st.policy.on {
            return Vec::new();
        }
        st.used
            .values()
            .filter(|w| {
                w.item.is_some() && covers(&w.scope.clone().unwrap_or_default(), project, kind)
            })
            .take(agora_protocol::share::MAX_PER_AGENT)
            .cloned()
            .collect()
    }

    /// ADR-0122 step 3b: the shared MCP servers an agent of `kind` in `project` gets at its launch, judged again first with today's
    /// trust and policy, their `{secret:NAME}` filled from this node's own store (`secret`). A server whose secret is missing here is
    /// not given and is said once (`share.secret_missing`). What was given is kept for the agent's `AgentStarted`.
    pub fn servers_for(
        &self,
        agent: &str,
        project: &str,
        kind: &str,
        secret: &dyn Fn(&str) -> Option<String>,
    ) -> Vec<agora_adapters::inject::SharedServer> {
        self.rejudge();
        let mut servers = Vec::new();
        let mut given = Vec::new();
        for w in self.for_launch(project, kind) {
            let Some(m) = w.item.as_ref().and_then(|i| i.mcp.clone()) else {
                continue;
            };
            let fill = |v: &[v1::ShareValue]| -> Result<Vec<(String, String)>, String> {
                v.iter()
                    .map(|x| Ok((x.name.clone(), fill_secrets(&x.value, secret)?)))
                    .collect()
            };
            let filled = fill(&m.env).and_then(|env| {
                let headers = fill(&m.headers)?;
                let args = m
                    .args
                    .iter()
                    .map(|a| fill_secrets(a, secret))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((env, headers, args))
            });
            match filled {
                Ok((env, headers, args)) => {
                    given.push(v1::SharedItemRef {
                        name: w.name.clone(),
                        item_hash: w.item_hash.clone(),
                    });
                    servers.push(agora_adapters::inject::SharedServer {
                        name: w.name.clone(),
                        command: m.command.clone(),
                        args,
                        env,
                        url: m.url.clone(),
                        headers,
                    });
                }
                Err(missing) => {
                    let mut st = self.st();
                    if st
                        .refused
                        .insert((w.name.clone(), format!("{}#secret", w.item_hash)))
                    {
                        self.out.lock().unwrap_or_else(|e| e.into_inner()).push(
                            Body::ShareItemRefused(v1::ShareItemRefused {
                                node_id: self.node_id.clone(),
                                name: w.name.clone(),
                                item_hash: w.item_hash.clone(),
                                code: "share.secret_missing".to_string(),
                                reason: missing,
                            }),
                        );
                    }
                }
            }
        }
        self.given
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(agent.to_string(), given);
        servers
    }

    /// What `agent` was given at its launch (`AgentStarted.shared_items`); nothing for an agent started without shared items.
    pub fn given(&self, agent: &str) -> Vec<v1::SharedItemRef> {
        self.given
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(agent)
            .cloned()
            .unwrap_or_default()
    }

    /// The file of the shared servers an agent was given (outside its worktree, readable by this user only).
    pub fn agent_file(&self, agent: &str) -> PathBuf {
        self.dir.join("agents").join(format!("{agent}.json"))
    }

    /// The agent ended: its file of shared servers (it may hold secrets) is removed, and what it was given forgotten.
    pub fn agent_ended(&self, agent: &str) {
        let _ = std::fs::remove_file(self.agent_file(agent));
        self.given
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(agent);
    }

    /// Facts to record since the last call.
    pub fn take_events(&self) -> Vec<Body> {
        std::mem::take(&mut *self.out.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Applies what the hub answered, and asks for the list when it is time. From the daemon's loop.
    pub fn tick(&self, now: Instant) {
        let came = std::mem::take(&mut *self.from_link.lock().unwrap_or_else(|e| e.into_inner()));
        for c in came {
            match c {
                FromLink::Listed(Ok(list)) => self.listed(list),
                // Not answered (the link was not up yet, or went down): asked again soon, not a whole poll later.
                FromLink::Listed(Err(_)) => {
                    let mut st = self.st();
                    st.next_poll = Some(st.next_poll.map_or(now + RETRY, |t| t.min(now + RETRY)));
                }
                FromLink::Got(name, hash, r) => {
                    self.st().asking.remove(&(name.clone(), hash));
                    if let Ok(w) = r {
                        if w.name == name {
                            self.take(*w);
                        }
                    }
                }
            }
        }
        let link = self.link.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(link) = link else {
            return;
        };
        {
            let mut st = self.st();
            if !st.policy.on || st.next_poll.is_some_and(|t| now < t) {
                return;
            }
            st.next_poll = Some(now + POLL);
        }
        // agora-ac's review of 7874ffba: a signer the owner untrusted or limited here stops counting at the next poll, not at the
        // item's next version.
        self.rejudge();
        let back = Arc::clone(&self.from_link);
        link.request(
            command_request::Command::ShareItemList(v1::ShareItemList {
                with_content: false,
            }),
            "",
            Box::new(move |r| {
                let r = r.map(|ok| match ok.result {
                    Some(command_ok::Result::ShareItemList(l)) => l.items,
                    _ => Vec::new(),
                });
                back.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(FromLink::Listed(r));
            }),
        );
    }

    /// The hub's list: a name no longer listed is dropped (the hub can narrow); a version not in use, not refused and not asked
    /// already is fetched.
    fn listed(&self, list: Vec<ShareItemInfo>) {
        let link = self.link.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut fetch = Vec::new();
        {
            let mut st = self.st();
            let names: BTreeSet<&str> = list.iter().map(|i| i.name.as_str()).collect();
            let gone: Vec<String> = st
                .used
                .keys()
                .filter(|n| !names.contains(n.as_str()))
                .cloned()
                .collect();
            for n in gone {
                st.used.remove(&n);
                let _ = std::fs::remove_file(self.dir.join(format!("{n}.pb")));
            }
            for i in &list {
                let key = (i.name.clone(), i.item_hash.clone());
                let in_use = st
                    .used
                    .get(&i.name)
                    .is_some_and(|w| w.item_hash == i.item_hash);
                let older = st.newest.get(&i.name).is_some_and(|n| *n > i.issued_ms);
                if in_use
                    || older
                    || !valid_name(&i.name, 64)
                    || st.refused.contains(&key)
                    || st.asking.contains(&key)
                {
                    continue;
                }
                st.asking.insert(key);
                fetch.push((i.name.clone(), i.item_hash.clone()));
            }
        }
        let Some(link) = link else {
            return;
        };
        for (name, hash) in fetch {
            let back = Arc::clone(&self.from_link);
            let (n, h) = (name.clone(), hash.clone());
            link.request(
                command_request::Command::ShareItemGet(v1::ShareItemGet {
                    name,
                    item_hash: hash,
                }),
                "",
                Box::new(move |r| {
                    let r = r.and_then(|ok| match ok.result {
                        Some(command_ok::Result::ShareItemGet(g)) => g
                            .signed
                            .map(Box::new)
                            .ok_or_else(|| "share.unknown".to_string()),
                        _ => Err("share.unknown".to_string()),
                    });
                    back.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(FromLink::Got(n, h, r));
                }),
            );
        }
    }

    /// Judges every item in use again with today's trust and policy: one whose signer no longer holds here (untrusted, out of its
    /// term, limited to projects) or that the policy no longer allows is unloaded and said (`signer_untrusted`, or the policy's
    /// reason). Called at every poll, and before a launch hands items out (step 3b).
    pub fn rejudge(&self) {
        let now_ms = crate::daemon::now_ms();
        let mut st = self.st();
        let words: Vec<ShareItemSigned> = st.used.values().cloned().collect();
        for w in words {
            let newest = st.newest.get(&w.name).copied();
            let Err(r) = judge(&w, newest, &st.policy, |signer, message, signature| {
                (self.vouch)(signer, message, signature, now_ms)
            }) else {
                continue;
            };
            let reason = if r.code == "approval.signature_invalid" {
                "signer_untrusted"
            } else {
                r.reason
            };
            st.used.remove(&w.name);
            let _ = std::fs::remove_file(self.dir.join(format!("{}.pb", w.name)));
            self.out
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(Body::ShareItemRefused(v1::ShareItemRefused {
                    node_id: self.node_id.clone(),
                    name: w.name.clone(),
                    item_hash: w.item_hash.clone(),
                    code: "share.refused".to_string(),
                    reason: reason.to_string(),
                }));
            // Said once: a later list naming the same version does not fetch it again.
            st.refused.insert((w.name, w.item_hash));
        }
    }

    /// Judges a word with this node's trust and policy; uses it, ends what it removes, or says why not (once per version).
    fn take(&self, w: ShareItemSigned) {
        let now_ms = crate::daemon::now_ms();
        let mut st = self.st();
        let newest = st.newest.get(&w.name).copied();
        let verdict = judge(&w, newest, &st.policy, |signer, message, signature| {
            (self.vouch)(signer, message, signature, now_ms)
        });
        match verdict {
            Ok(()) => {
                st.newest.insert(w.name.clone(), w.issued_ms);
                let _ = std::fs::create_dir_all(&self.dir);
                if let Ok(text) = serde_json::to_string(&st.newest) {
                    let _ =
                        agora_platform::write_private(&self.dir.join(NEWEST_FILE), text.as_bytes());
                }
                let file = self.dir.join(format!("{}.pb", w.name));
                if w.item.is_some() {
                    let _ = agora_platform::write_private(&file, &w.encode_to_vec());
                    st.used.insert(w.name.clone(), w);
                } else {
                    let _ = std::fs::remove_file(&file);
                    st.used.remove(&w.name);
                }
            }
            Err(r) => {
                // A word this node cannot trust (its signature, its bytes, an older date) changes nothing here: acting on it would let
                // whoever passes it on take an item away. Only the owner's newer, trusted word that this node's policy will not run
                // ends the version in use (the owner replaced it; the old one is not run instead).
                if matches!(
                    r.reason,
                    "policy_off" | "command_not_allowed" | "url_not_allowed"
                ) {
                    st.used.remove(&w.name);
                    let _ = std::fs::remove_file(self.dir.join(format!("{}.pb", w.name)));
                }
                if st.refused.insert((w.name.clone(), w.item_hash.clone())) {
                    self.out.lock().unwrap_or_else(|e| e.into_inner()).push(
                        Body::ShareItemRefused(v1::ShareItemRefused {
                            node_id: self.node_id.clone(),
                            name: w.name.clone(),
                            item_hash: w.item_hash.clone(),
                            code: r.code.to_string(),
                            reason: r.reason.to_string(),
                        }),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_protocol::v1::{McpServerDef, ShareItem, ShareItemKind, ShareScope};

    fn item(name: &str, command: &str, url: &str) -> ShareItem {
        ShareItem {
            name: name.into(),
            kind: ShareItemKind::McpServer as i32,
            mcp: Some(McpServerDef {
                command: command.into(),
                url: url.into(),
                ..Default::default()
            }),
            skill: None,
        }
    }

    /// A word as the owner's device `web-1` would sign it; the fake trust below accepts exactly what `web-1` signs over the bytes.
    fn word(
        name: &str,
        item: Option<ShareItem>,
        issued_ms: u64,
        scope: ShareScope,
    ) -> ShareItemSigned {
        let hash = item.as_ref().map(item_hash).unwrap_or_default();
        let action = if item.is_some() {
            Action::Publish
        } else {
            Action::Remove
        };
        let message = signed_message(action, name, &hash, &scope, issued_ms);
        ShareItemSigned {
            item,
            scope: Some(scope),
            issued_ms,
            item_hash: hash,
            signer_device_id: "web-1".into(),
            signature: sha256_hex(&message).into_bytes(),
            name: name.into(),
        }
    }

    /// Trust that vouches for `web-1` only, and only for the bytes it signed.
    fn trusted(signer: &str, message: &[u8], signature: &[u8]) -> bool {
        signer == "web-1" && signature == sha256_hex(message).as_bytes()
    }

    #[test]
    fn a_word_is_used_only_when_this_nodes_trust_and_policy_allow_it() {
        let on = SharePolicy::default();
        let w = word("db", Some(item("db", "npx", "")), 10, ShareScope::default());
        assert_eq!(judge(&w, None, &on, trusted), Ok(()));
        assert_eq!(
            judge(&w, Some(10), &on, trusted),
            Ok(()),
            "the same word again"
        );
        assert_eq!(
            judge(&w, Some(11), &on, trusted).unwrap_err().reason,
            "older"
        );
        // Another device, or the bytes changed after signing.
        let mut other = w.clone();
        other.signer_device_id = "web-2".into();
        assert_eq!(
            judge(&other, None, &on, trusted).unwrap_err().code,
            "approval.signature_invalid"
        );
        let mut widened = w.clone();
        widened.scope = Some(ShareScope {
            project_id: "p".into(),
            kinds: Vec::new(),
        });
        assert_eq!(
            judge(&widened, None, &on, trusted).unwrap_err().code,
            "approval.signature_invalid"
        );
        let mut swapped = w.clone();
        swapped.item = Some(item("db", "sh", ""));
        assert_eq!(
            judge(&swapped, None, &on, trusted).unwrap_err().reason,
            "invalid"
        );
        // This node's policy is the ceiling.
        let off = SharePolicy {
            on: false,
            ..SharePolicy::default()
        };
        assert_eq!(
            judge(&w, None, &off, trusted).unwrap_err().reason,
            "policy_off"
        );
        let only_uvx = SharePolicy {
            mcp_commands: Some(["uvx".to_string()].into()),
            ..SharePolicy::default()
        };
        assert_eq!(
            judge(&w, None, &only_uvx, trusted).unwrap_err().reason,
            "command_not_allowed"
        );
        let remote = word(
            "docs",
            Some(item("docs", "", "https://mcp.example.com")),
            10,
            ShareScope::default(),
        );
        assert_eq!(
            judge(&remote, None, &only_uvx, trusted),
            Ok(()),
            "a remote server runs nothing here"
        );
        // A removal needs only trust and to be newer.
        assert_eq!(
            judge(
                &word("db", None, 12, ShareScope::default()),
                Some(10),
                &off,
                trusted
            ),
            Ok(())
        );
    }

    #[test]
    fn a_scope_covers_its_project_and_kinds() {
        let all = ShareScope::default();
        assert!(covers(&all, "p", "codex"));
        let one = ShareScope {
            project_id: "p".into(),
            kinds: vec!["claude-code".into()],
        };
        assert!(covers(&one, "p", "claude-code"));
        assert!(!covers(&one, "q", "claude-code") && !covers(&one, "p", "codex"));
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("agora-shared-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn open(dir: &Path, policy: SharePolicy) -> SharedItems {
        SharedItems::open_with(
            dir,
            "blackview",
            Box::new(|s, m, sig, _| trusted(s, m, sig)),
            policy,
        )
    }

    /// What a node used and the newest word it saw survive a restart: a replayed older word is refused then too, a removal ends the
    /// item, and a refusal is said once per version.
    #[test]
    fn used_items_and_the_newest_words_survive_a_restart_and_a_refusal_is_said_once() {
        let d = temp("restart");
        let s = open(&d, SharePolicy::default());
        let scope = ShareScope {
            project_id: String::new(),
            kinds: vec!["claude-code".into()],
        };
        s.take(word("db", Some(item("db", "npx", "")), 20, scope.clone()));
        assert_eq!(s.for_launch("p", "claude-code").len(), 1);
        assert!(s.for_launch("p", "codex").is_empty(), "outside its scope");
        drop(s);
        let s = open(&d, SharePolicy::default());
        assert_eq!(s.for_launch("p", "claude-code").len(), 1, "kept on disk");
        s.take(word("db", Some(item("db", "sh", "")), 19, scope.clone()));
        s.take(word("db", Some(item("db", "sh", "")), 19, scope.clone()));
        let said = s.take_events();
        assert_eq!(said.len(), 1, "once per version");
        assert!(
            matches!(&said[0], Body::ShareItemRefused(r) if r.reason == "older" && r.node_id == "blackview")
        );
        assert_eq!(
            s.for_launch("p", "claude-code")[0].item_hash,
            item_hash(&item("db", "npx", "")),
            "the newer one stays"
        );
        s.take(word("db", None, 21, ShareScope::default()));
        assert!(s.for_launch("p", "claude-code").is_empty());
        drop(s);
        let s = open(&d, SharePolicy::default());
        assert!(
            s.for_launch("p", "claude-code").is_empty(),
            "the removal is kept"
        );
        s.take(word("db", Some(item("db", "npx", "")), 20, scope));
        assert!(
            s.for_launch("p", "claude-code").is_empty(),
            "the word before the removal cannot come back"
        );
        // A newer version this node's policy will not run ends the one in use.
        let s2 = open(
            &temp("policy"),
            SharePolicy {
                mcp_commands: Some(["npx".to_string()].into()),
                ..SharePolicy::default()
            },
        );
        s2.take(word(
            "x",
            Some(item("x", "npx", "")),
            1,
            ShareScope::default(),
        ));
        s2.take(word(
            "x",
            Some(item("x", "sh", "")),
            2,
            ShareScope::default(),
        ));
        assert!(s2.for_launch("p", "codex").is_empty());
        // Turning the policy off gives no item at all.
        s.set_policy(SharePolicy {
            on: false,
            ..SharePolicy::default()
        });
        assert!(s.for_launch("p", "claude-code").is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A signer the owner untrusts here stops counting at the next look, not at the item's next version.
    #[test]
    fn an_item_whose_signer_is_no_longer_trusted_is_unloaded_and_said() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let d = temp("untrust");
        let trust = Arc::new(AtomicBool::new(true));
        let t = Arc::clone(&trust);
        let s = SharedItems::open_with(
            &d,
            "blackview",
            Box::new(move |signer, m, sig, _| t.load(Ordering::SeqCst) && trusted(signer, m, sig)),
            SharePolicy::default(),
        );
        s.take(word(
            "db",
            Some(item("db", "npx", "")),
            20,
            ShareScope::default(),
        ));
        s.rejudge();
        assert_eq!(s.for_launch("p", "codex").len(), 1, "still trusted");
        assert!(s.take_events().is_empty());
        trust.store(false, Ordering::SeqCst);
        s.rejudge();
        assert!(s.for_launch("p", "codex").is_empty());
        let said = s.take_events();
        assert!(matches!(&said[..], [Body::ShareItemRefused(r)] if r.reason == "signer_untrusted"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_node_can_forbid_shared_servers_at_a_url() {
        let no_urls = SharePolicy {
            mcp_urls: false,
            ..SharePolicy::default()
        };
        let remote = word(
            "docs",
            Some(item("docs", "", "https://mcp.example.com")),
            10,
            ShareScope::default(),
        );
        assert_eq!(
            judge(&remote, None, &no_urls, trusted).unwrap_err().reason,
            "url_not_allowed"
        );
        let program = word("db", Some(item("db", "npx", "")), 10, ShareScope::default());
        assert_eq!(judge(&program, None, &no_urls, trusted), Ok(()));
    }

    #[test]
    fn secrets_are_filled_from_this_nodes_store_and_a_missing_one_is_named() {
        let store = |n: &str| (n == "DB_URL").then(|| "postgres://db".to_string());
        assert_eq!(fill_secrets("plain", &store), Ok("plain".into()));
        assert_eq!(
            fill_secrets("{secret:DB_URL}?ssl=1", &store),
            Ok("postgres://db?ssl=1".into())
        );
        assert_eq!(
            fill_secrets("Bearer {secret:TOKEN}", &store),
            Err("TOKEN".into())
        );
        assert_eq!(
            fill_secrets("{secret:../x}", &store),
            Err("../x".into()),
            "not a plain name"
        );
        assert_eq!(
            fill_secrets("{secret:open", &store),
            Ok("{secret:open".into()),
            "an unclosed brace is text"
        );
    }

    /// At launch an agent gets the servers in its scope with their secrets filled; one whose secret is missing here is left out and
    /// said once; what it got is kept for its start, and forgotten when it ends.
    #[test]
    fn a_launch_gets_its_servers_with_secrets_and_a_missing_secret_leaves_one_out() {
        let d = temp("launch");
        let s = open(&d, SharePolicy::default());
        let mut db = item("db", "npx", "");
        db.mcp.as_mut().unwrap().env = vec![v1::ShareValue {
            name: "DB_URL".into(),
            value: "{secret:DB_URL}".into(),
        }];
        let mut docs = item("docs", "", "https://mcp.example.com");
        docs.mcp.as_mut().unwrap().headers = vec![v1::ShareValue {
            name: "Authorization".into(),
            value: "Bearer {secret:DOCS}".into(),
        }];
        s.take(word("db", Some(db), 1, ShareScope::default()));
        s.take(word("docs", Some(docs), 1, ShareScope::default()));
        let store = |n: &str| (n == "DB_URL").then(|| "postgres://db".to_string());
        let got = s.servers_for("agent-1", "p", "claude-code", &store);
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0].env,
            [("DB_URL".to_string(), "postgres://db".to_string())]
        );
        assert_eq!(
            s.given("agent-1")
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>(),
            ["db"]
        );
        s.servers_for("agent-2", "p", "claude-code", &store);
        let said = s.take_events();
        assert!(
            matches!(&said[..], [Body::ShareItemRefused(r)] if r.code == "share.secret_missing" && r.reason == "DOCS"),
            "once: {said:?}"
        );
        s.agent_ended("agent-1");
        assert!(s.given("agent-1").is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The hub can only narrow: a name it stops listing is dropped here.
    #[test]
    fn a_name_the_hub_stops_listing_is_dropped() {
        let d = temp("narrow");
        let s = open(&d, SharePolicy::default());
        s.take(word(
            "db",
            Some(item("db", "npx", "")),
            20,
            ShareScope::default(),
        ));
        assert_eq!(s.for_launch("p", "codex").len(), 1);
        s.listed(Vec::new());
        assert!(s.for_launch("p", "codex").is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
