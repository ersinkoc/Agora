//! User-signed grants for the commands a client sends through the hub (ADR-0041).
//!
//! A node whose policy has a `signed_commands` line refuses those commands when they arrive routed through the hub, unless
//! a live grant covers them. A grant is signed by a trusted device (the devices of ADR-0028) and verified here, so a hub
//! that forges one fails the check. Grants live in memory only: a restart ends them on purpose. Commands that arrive on the
//! node's own control socket never pass through here (the person at the machine needs no grant).
//!
//! A grant lasts only as long as the trust of the device that signed it (audit F270): when the trusted devices change (a device
//! removed, revoked, re-trusted, its limits narrowed), every live grant is checked again before the next command it would cover, and
//! one whose signer no longer vouches for it, in the same term of trust and within its limits, ends for good. An endorsed device's
//! grant also ends when the device's trust runs out.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use agora_protocol::v1::{self, command_request::Command as Cmd};
use agora_protocol::{Error, ErrorCode};

use crate::approvals::{NodeApprovals, Term};
use crate::control::{agent_of_terminal, command_name};

/// A grant lasts at most this long after it is issued (8 hours).
pub const MAX_TTL_MS: u64 = 8 * 60 * 60 * 1000;
/// Slack for the clock difference between the signing device and this node.
const SKEW_MS: u64 = 60 * 1000;
const MAX_LIVE: usize = 64;
const MAX_COMMANDS: usize = 32;
const SPENT_MEMORY: usize = 4096;

/// The exact bytes a device signs for a grant: a domain tag, then node id, grant id, scope, the commands joined by commas
/// and the expiry in decimal milliseconds, NUL-separated.
pub fn signed_message(
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

/// Milliseconds since the Unix epoch (a clock before it counts as 0, which expires every grant).
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Which routed commands need a grant. Patterns are exact command names or a prefix ending in `*`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignedCommands(pub Vec<String>);

fn matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => pattern == name,
    }
}

impl SignedCommands {
    /// What a node with limited devices (`device.<id>.projects`, ADR-0049) needs signed when its owner wrote no `signed_commands`
    /// line: everything that changes something, so the device limits bind every such command and not only the ones the owner listed.
    pub fn team_default() -> Self {
        Self(
            [
                "agent.start",
                "agent.stop",
                "agent.prompt",
                "agent.set_mode",
                "agent.set_model",
                // ADR-0090: handled by this node; listed so a limited device's grant covers them (a MODE-category option is a mode
                // switch).
                "agent.set_config",
                "agent.cancel_turn",
                // ADR-0095: it claims a task and prompts an agent, like `agent.prompt`.
                "task.dispatch",
                "terminal.open",
                "terminal.input",
                "files.write",
                "worktree.create",
                "worktree.remove",
            ]
            .map(String::from)
            .to_vec(),
        )
    }

    pub fn requires(&self, name: &str) -> bool {
        self.0.iter().any(|p| matches(p, name))
    }
}

#[derive(Default)]
struct State {
    live: Vec<Live>,
    spent: VecDeque<String>,
}

/// A live grant, with what it takes to check again that its signer still vouches for it (audit F270).
#[derive(Clone)]
struct Live {
    info: v1::GrantInfo,
    signature: Vec<u8>,
    /// The signer's term of trust when the grant was made: a later term (the device removed and trusted again) does not carry it.
    since: Option<u64>,
    /// When the signer's trust ends by itself (an endorsed device), as last checked.
    until: Option<u64>,
    /// The trust generation it was last checked under ([`NodeApprovals::trust_generation`]).
    checked: u64,
}

impl Live {
    /// Still to be checked against the trust held at `generation`: the trust changed since, or the signer's term ran out.
    fn stale(&self, generation: u64, now_ms: u64) -> bool {
        self.checked < generation || self.until.is_some_and(|u| now_ms >= u)
    }
}

type Resolver = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// What a directory action lands in, for a device limited to some projects (ADR-0049, ADR-0052, ADR-0053).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place<'a> {
    /// A folder path as the request names it, or for `directory.inspect` also a registered project's id or name.
    Folder(&'a str),
    /// The node's own id of an agent (`agent.resume`).
    Agent(&'a str),
}

/// The project a [`Place`] belongs to by the node's own records (never by what the request or the folder claims), or `None` when
/// the node does not know it as part of any project.
pub type PlaceResolver = Box<dyn Fn(Place<'_>) -> Option<String> + Send + Sync>;

/// A signed action's nonce could not be spent (used already, the store is full, or its file failed).
#[derive(Debug)]
struct NonceSpent;

pub struct Grants {
    node_id: String,
    approvals: Arc<NodeApprovals>,
    policy: SignedCommands,
    state: Mutex<State>,
    directory_nonces: Mutex<std::collections::BTreeMap<String, u64>>,
    directory_nonce_file: Option<std::path::PathBuf>,
    /// Agent id to project id, for grants scoped to a project.
    project_of: Mutex<Option<Resolver>>,
    /// What each trusted device may sign (ADR-0049).
    limits: Arc<crate::limits::DeviceLimits>,
    /// Which project a directory action lands in, for limited devices. Unset: a limited device can take no directory action.
    places: Mutex<Option<PlaceResolver>>,
    /// #1133: whether a start of a kind must be signed (the node's loaded manifests say). Unset: every `registry-*` kind.
    signed_starts: Mutex<Option<KindCheck>>,
}

/// Whether something holds for an agent kind, by the node's own manifests.
pub type KindCheck = Box<dyn Fn(&str) -> bool + Send + Sync>;

/// What a command is about, as far as a grant's scope can tell.
#[derive(Default)]
struct Subject {
    agent: Option<String>,
    project: Option<String>,
}

fn subject(c: &Cmd) -> Subject {
    let mut s = Subject::default();
    match c {
        Cmd::AgentStart(a) => s.project = Some(a.project_id.clone()),
        Cmd::AgentStop(a) => s.agent = Some(a.agent_id.clone()),
        Cmd::AgentPrompt(a) => s.agent = Some(a.agent_id.clone()),
        Cmd::AgentSetMode(a) => s.agent = Some(a.agent_id.clone()),
        Cmd::AgentSetModel(a) => s.agent = Some(a.agent_id.clone()),
        Cmd::AgentSetConfig(a) => s.agent = Some(a.agent_id.clone()),
        Cmd::AgentCancelTurn(a) => s.agent = Some(a.agent_id.clone()),
        // ADR-0095: the agent, so a project grant is matched against the agent's real project, never the command's `project_id`.
        Cmd::TaskDispatch(a) => s.agent = Some(a.agent_id.clone()),
        // ADR-0108: the hub passes the person's forget on with the node's own id of the agent; an agent grant covers it.
        Cmd::AgentForget(a) => s.agent = Some(a.agent_id.clone()),
        Cmd::TerminalInput(t) => {
            s.agent = agent_of_terminal(&t.terminal_id).map(|a| a.as_str().to_string());
        }
        Cmd::TerminalResize(t) => {
            s.agent = agent_of_terminal(&t.terminal_id).map(|a| a.as_str().to_string());
        }
        // Watching names its agent, so a grant for that agent (or its project) covers it, as it covers typing into it.
        Cmd::TerminalSubscribe(t) => {
            s.agent = agent_of_terminal(&t.terminal_id).map(|a| a.as_str().to_string());
        }
        Cmd::ChatSubscribe(c) => s.agent = Some(c.agent_id.clone()),
        // Commands about a worktree or a new terminal name no agent: only a `node` grant covers them.
        _ => {}
    }
    s
}

fn valid_scope(scope: &str) -> bool {
    scope == "node"
        || scope
            .strip_prefix("agent:")
            .or_else(|| scope.strip_prefix("project:"))
            .is_some_and(|id| !id.is_empty() && id.len() <= 128 && !id.contains('\0'))
}

fn invalid(field: &str) -> Error {
    Error::new(ErrorCode::ProtocolInvalidMessage).with("field", field)
}

impl Grants {
    pub fn new(node_id: &str, approvals: Arc<NodeApprovals>, policy: SignedCommands) -> Self {
        Self {
            node_id: node_id.to_string(),
            approvals,
            policy,
            state: Mutex::new(State::default()),
            directory_nonces: Mutex::new(std::collections::BTreeMap::new()),
            directory_nonce_file: None,
            project_of: Mutex::new(None),
            limits: Arc::new(crate::limits::DeviceLimits::default()),
            places: Mutex::new(None),
            signed_starts: Mutex::new(None),
        }
    }

    /// Tells grants which project a folder or an agent belongs to, so a device limited to some projects can take directory actions
    /// inside them.
    /// #1133: how to tell a kind whose start must be signed (`crate::manifests::needs_signed_start` over the node's kinds).
    pub fn set_signed_start_check(&self, f: KindCheck) {
        *self.signed_starts.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    /// #1133 ("startable, but always signed"): whether a start of `kind` that reaches the node through the hub must carry a person's
    /// device signature (a routed `AgentLaunch`), so that an unsigned `AgentStart` of it is refused whatever `signed_commands` says.
    /// Without a check set, every registry kind (`registry-*`) does: the stricter reading.
    pub fn start_needs_signature(&self, kind: &str) -> bool {
        match &*self.signed_starts.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(f) => f(kind),
            None => kind.starts_with(crate::manifests::REGISTRY_PREFIX),
        }
    }

    pub fn set_place_resolver(&self, f: PlaceResolver) {
        *self.places.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    /// May a device limited to some projects take this directory action? Only inside a project it may act in, judged by the node's
    /// own records of where that project is: a folder the node already knows for that project (a registered checkout, or a folder an
    /// agent of that project was launched in) or a folder inside one; for a launch also the project the request was signed for, which
    /// the launch itself then checks against the folder's identity. Never a new identity (`create_project_id`), never a changed one
    /// (`accept_identity_change`), never the home folder or a folder the node knows for no project or for another project.
    fn limited_directory_allowed(&self, device: &str, c: &Cmd) -> bool {
        let guard = self.places.lock().unwrap_or_else(|e| e.into_inner());
        let Some(place_of) = guard.as_ref() else {
            return false;
        };
        let (place, signed_project) = match c {
            Cmd::DirectoryInspect(d) if !d.create_project_id => (Place::Folder(&d.path), None),
            Cmd::DirectoryList(d) if !d.path.is_empty() => (Place::Folder(&d.path), None),
            Cmd::AgentLaunch(a) if !a.accept_identity_change => {
                (Place::Folder(&a.path), Some(a.project_id.as_str()))
            }
            Cmd::AgentResume(a) => (Place::Agent(&a.agent_id), None),
            _ => return false,
        };
        let Some(project) = place_of(place) else {
            return false;
        };
        // ADR-0115: signed for the folder's id in an older spelling is the same project.
        signed_project.is_none_or(|p| agora_core::same_project(p, &project))
            && self.limits.allows_project(device, &project)
    }

    pub fn with_directory_nonce_file(mut self, path: std::path::PathBuf) -> Self {
        self.directory_nonce_file = Some(path);
        self
    }

    /// Directory actions are always exact-path, short-lived, owner-signed and single-use on the routed path.
    pub fn directory_action(&self, c: &Cmd, now: u64) -> Result<(), Error> {
        let (launch_options, confirmed): (&[String], &[String]) = match c {
            Cmd::AgentLaunch(a) => (&a.launch_options, &a.confirmed_launch_options),
            _ => (&[], &[]),
        };
        // ADR-0066: the settings, in the order the request carries them (the order that was signed).
        let settings: Vec<(String, String)> = match c {
            Cmd::AgentLaunch(a) => a
                .settings
                .iter()
                .map(|st| (st.id.clone(), st.value.clone()))
                .collect(),
            _ => Vec::new(),
        };
        let (auth, op, path, kind, project, option, identity_change) = match c {
            Cmd::DirectoryInspect(d) => (
                d.authorization.as_ref(),
                "inspect",
                d.path.as_str(),
                "",
                "",
                d.create_project_id,
                false,
            ),
            Cmd::DirectoryList(d) => (
                d.authorization.as_ref(),
                "list",
                d.path.as_str(),
                "",
                "",
                false,
                false,
            ),
            Cmd::AgentLaunch(a) => (
                a.authorization.as_ref(),
                "launch",
                a.path.as_str(),
                a.kind.as_str(),
                a.project_id.as_str(),
                a.separate_worktree,
                a.accept_identity_change,
            ),
            // ADR-0060: the agent id stands in the path field; the node takes its folder, kind and project from its own ledger.
            Cmd::AgentResume(a) => (
                a.authorization.as_ref(),
                "resume",
                a.agent_id.as_str(),
                "",
                "",
                a.new_session,
                false,
            ),
            _ => return Ok(()),
        };
        let bad =
            || Error::new(ErrorCode::ApprovalSignatureInvalid).with("approval_id", "directory");
        let a = auth.ok_or_else(bad)?;
        if [path, kind, project, a.nonce.as_str(), a.device_id.as_str()]
            .iter()
            .any(|s| s.contains('\0') || s.len() > 4096)
            // The lists are signed length-prefixed, so their contents cannot be confused; the node checks each id afterwards
            // (`agent.option_invalid`). Only their size is bounded here.
            || launch_options.len() > 16
            || confirmed.len() > 16
            || launch_options.iter().chain(confirmed).any(|o| o.len() > 64)
            || settings.len() > agora_adapters::manifest::MAX_LAUNCH_SETTINGS
            || settings
                .iter()
                .any(|(id, value)| id.len() > 64 || value.len() > 128)
            || a.nonce.len() != 32
            || !a.nonce.bytes().all(|b| b.is_ascii_hexdigit())
            || a.expires_at_ms <= now
            || a.expires_at_ms > now.saturating_add(5 * 60 * 1000)
        {
            return Err(bad());
        }
        // ADR-0059: chosen launch options are signed too, under their own domain (a hub can neither add nor remove one).
        // ADR-0066: chosen settings too, under a domain of their own: a launch with settings verifies only over `directory-action-v4`.
        let msg = if !settings.is_empty() {
            crate::directory::launch_signed_message_v4(
                &self.node_id,
                path,
                kind,
                project,
                option,
                identity_change,
                launch_options,
                confirmed,
                &settings,
                &a.nonce,
                a.expires_at_ms,
            )
        } else if launch_options.is_empty() && confirmed.is_empty() {
            crate::directory::signed_message(
                &self.node_id,
                op,
                path,
                kind,
                project,
                option,
                identity_change,
                &a.nonce,
                a.expires_at_ms,
            )
        } else {
            crate::directory::launch_signed_message(
                &self.node_id,
                path,
                kind,
                project,
                option,
                identity_change,
                launch_options,
                confirmed,
                &a.nonce,
                a.expires_at_ms,
            )
        };
        if !self
            .approvals
            .verify_device(&a.device_id, &msg, &a.signature)
        {
            return Err(bad());
        }
        // Checked after the signature, so only a device this node trusts learns whether a folder lies in its projects.
        if self.limits.is_limited(&a.device_id) && !self.limited_directory_allowed(&a.device_id, c)
        {
            return Err(Error::new(ErrorCode::AuthForbidden).with("action", "directory"));
        }
        self.spend_nonce(&a.device_id, &a.nonce, a.expires_at_ms, now)
            .map_err(|NonceSpent| bad())
    }

    /// ADR-0088: checks a `PreviewOpen` the hub routed here: a device this node trusts signed exactly this open (this node, this port,
    /// this lifetime) under `preview-action-v1`, the signature is unexpired and at most 5 minutes ahead, and its nonce is new (kept,
    /// on disk, until the signature's expiry). The signing device's id, which the forward names as its creator. A device the owner
    /// limited to some projects opens no preview (a forward reaches the whole machine's loopback, not a project). A grant never
    /// stands in for the signature.
    pub fn preview_action(
        &self,
        open: &agora_protocol::v1::PreviewOpen,
        now: u64,
    ) -> Result<String, Error> {
        let bad = || Error::new(ErrorCode::ApprovalSignatureInvalid).with("approval_id", "preview");
        let a = open.authorization.as_ref().ok_or_else(bad)?;
        if a.device_id.is_empty()
            || a.device_id.len() > 4096
            || a.device_id.contains('\0')
            || a.nonce.len() != 32
            || !a.nonce.bytes().all(|b| b.is_ascii_hexdigit())
            || a.expires_at_ms <= now
            || a.expires_at_ms > now.saturating_add(5 * 60 * 1000)
        {
            return Err(bad());
        }
        let msg = crate::preview::signed_message(
            &self.node_id,
            open.port,
            open.expires_seconds,
            &a.nonce,
            a.expires_at_ms,
        );
        if !self
            .approvals
            .verify_device(&a.device_id, &msg, &a.signature)
        {
            return Err(bad());
        }
        // After the signature, as for a directory action: only a trusted device learns that it is limited.
        if self.limits.is_limited(&a.device_id) {
            return Err(Error::new(ErrorCode::AuthForbidden).with("action", "preview"));
        }
        self.spend_nonce(&a.device_id, &a.nonce, a.expires_at_ms, now)
            .map_err(|NonceSpent| bad())?;
        Ok(a.device_id.clone())
    }

    /// Records a signed action's nonce as used until `expires_at_ms`, in memory and (when the node has one) in the nonce file, so a
    /// restart does not forget it. `Err` when it was used already, the store is full, or the file cannot be read or written.
    fn spend_nonce(
        &self,
        device: &str,
        nonce: &str,
        expires_at_ms: u64,
        now: u64,
    ) -> Result<(), NonceSpent> {
        let bad = || NonceSpent;
        let key = format!("{device}:{nonce}");
        let mut used = self
            .directory_nonces
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(path) = &self.directory_nonce_file {
            match std::fs::read(path) {
                Ok(bytes) => {
                    if bytes.len() > 2 * 1024 * 1024 {
                        return Err(bad());
                    }
                    let saved: std::collections::BTreeMap<String, u64> =
                        serde_json::from_slice(&bytes).map_err(|_| bad())?;
                    used.extend(saved);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(bad()),
            }
        }
        used.retain(|_, exp| *exp >= now);
        if used.contains_key(&key) || used.len() >= 4096 {
            return Err(bad());
        }
        used.insert(key, expires_at_ms);
        if let Some(path) = &self.directory_nonce_file {
            let bytes = serde_json::to_vec(&*used).map_err(|_| bad())?;
            agora_platform::write_private(path, &bytes).map_err(|_| bad())?;
        }
        Ok(())
    }

    /// Limits what each device may sign (`device.<id>.projects` in the policy file; ADR-0049).
    pub fn with_limits(mut self, limits: Arc<crate::limits::DeviceLimits>) -> Self {
        self.limits = limits;
        self
    }

    /// Tells grants how to find an agent's project (a grant scoped to a project covers that project's agents).
    pub fn set_project_resolver(&self, f: Resolver) {
        *self.project_of.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    fn st(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Does a routed command need a grant at all on this node?
    pub fn policy(&self) -> &SignedCommands {
        &self.policy
    }

    /// Accepts a grant signed by a trusted device. Nothing changes unless every check passes.
    pub fn create(&self, g: &v1::GrantCreate, now_ms: u64) -> Result<(), Error> {
        if g.grant_id.is_empty() || g.grant_id.len() > 128 || g.grant_id.contains('\0') {
            return Err(invalid("grant_id"));
        }
        if !valid_scope(&g.scope) {
            return Err(invalid("scope"));
        }
        if g.commands.is_empty()
            || g.commands.len() > MAX_COMMANDS
            || g.commands
                .iter()
                .any(|c| c.is_empty() || c.len() > 64 || c.contains([',', '\0']))
        {
            return Err(invalid("commands"));
        }
        let bad = || {
            Error::new(ErrorCode::ApprovalSignatureInvalid).with("approval_id", g.grant_id.as_str())
        };
        // Signature first: whether the grant is expired or a replay is said only to a device this node trusts.
        let message = signed_message(
            &self.node_id,
            &g.grant_id,
            &g.scope,
            &g.commands,
            g.expires_at_ms,
        );
        let Some((checked, term)) =
            self.approvals
                .verify_device_term(&g.signer_device_id, &message, &g.signature)
        else {
            return Err(bad());
        };
        // A device the policy limits to some projects cannot grant beyond them, whatever it signed.
        if !self.limits.allows_scope(&g.signer_device_id, &g.scope) {
            return Err(Error::new(ErrorCode::AuthForbidden).with("action", "grant.create"));
        }
        if g.expires_at_ms <= now_ms {
            return Err(
                Error::new(ErrorCode::ApprovalExpired).with("approval_id", g.grant_id.as_str())
            );
        }
        if g.expires_at_ms > now_ms.saturating_add(MAX_TTL_MS + SKEW_MS) {
            return Err(invalid("expires_at_ms"));
        }
        let mut st = self.st();
        st.live.retain(|x| x.info.expires_at_ms > now_ms);
        if st.spent.iter().any(|s| *s == g.grant_id) || st.live.len() >= MAX_LIVE {
            return Err(bad());
        }
        st.spent.push_back(g.grant_id.clone());
        while st.spent.len() > SPENT_MEMORY {
            st.spent.pop_front();
        }
        st.live.push(Live {
            info: v1::GrantInfo {
                grant_id: g.grant_id.clone(),
                scope: g.scope.clone(),
                commands: g.commands.clone(),
                expires_at_ms: g.expires_at_ms,
                signer_device_id: g.signer_device_id.clone(),
                kind: String::new(),
                folder: String::new(),
                remaining: 0,
            },
            signature: g.signature.clone(),
            since: term.since,
            until: term.until,
            checked,
        });
        Ok(())
    }

    /// Checks again every live grant made or last checked under older trust, or whose signer's term ran out, and ends each one its
    /// signer no longer vouches for (audit F270). Returns the trust generation the live grants now stand under, or `None` when there
    /// are none (then nothing is read). A grant that fails ends for good: trusting its signer again later does not bring it back.
    fn recheck(&self, now_ms: u64) -> Option<u64> {
        if self.st().live.is_empty() {
            return None;
        }
        // One read of the trusted-devices file (compared with what was read last); signatures are checked only after a change.
        let generation = self.approvals.trust_generation();
        let stale: Vec<Live> = self
            .st()
            .live
            .iter()
            .filter(|g| g.stale(generation, now_ms))
            .cloned()
            .collect();
        if stale.is_empty() {
            return Some(generation);
        }
        // Checked without holding the grants: the limits may ask the supervisor for an agent's project.
        let verdicts: Vec<(String, Option<(u64, Term)>)> = stale
            .into_iter()
            .map(|g| {
                let verdict = self.still_vouched(&g, now_ms);
                (g.info.grant_id, verdict)
            })
            .collect();
        let mut st = self.st();
        for (id, verdict) in verdicts {
            match verdict {
                Some((at, term)) => {
                    if let Some(x) = st.live.iter_mut().find(|x| x.info.grant_id == id) {
                        x.checked = x.checked.max(at);
                        x.until = term.until;
                    }
                }
                None => st.live.retain(|x| x.info.grant_id != id),
            }
        }
        Some(generation)
    }

    /// Does the grant's signer still vouch for it: its signature verifies under the devices trusted now, in the term it was made in,
    /// before that term ends, and the device may still sign its scope (ADR-0049)? A check, not a use: nothing is renewed.
    fn still_vouched(&self, g: &Live, now_ms: u64) -> Option<(u64, Term)> {
        let i = &g.info;
        let message = signed_message(
            &self.node_id,
            &i.grant_id,
            &i.scope,
            &i.commands,
            i.expires_at_ms,
        );
        let (at, term) = self
            .approvals
            .vouches(&i.signer_device_id, &message, &g.signature)?;
        let holds = term.since == g.since
            && term.until.is_none_or(|u| now_ms < u)
            && self.limits.allows_scope(&i.signer_device_id, &i.scope);
        holds.then_some((at, term))
    }

    /// Live grants, then live helper grants (ADR-0057: `mesh.agent_start` for one agent, ending with it).
    pub fn list(&self, now_ms: u64) -> Vec<v1::GrantInfo> {
        self.recheck(now_ms);
        let mut out: Vec<v1::GrantInfo> = {
            let mut st = self.st();
            st.live.retain(|x| x.info.expires_at_ms > now_ms);
            st.live.iter().map(|x| x.info.clone()).collect()
        };
        out.extend(
            self.approvals
                .helper_grants()
                .into_iter()
                .map(|g| v1::GrantInfo {
                    grant_id: g.id,
                    scope: format!("agent:{}", g.agent.as_str()),
                    commands: vec!["mesh.agent_start".into()],
                    expires_at_ms: 0,
                    signer_device_id: g.signer_device_id,
                    kind: g.kind,
                    folder: g.folder,
                    remaining: g.remaining,
                }),
        );
        out
    }

    /// Ends a grant, or a helper grant by its approval id. An id that is not live is not an error (ending access is always allowed
    /// and idempotent).
    pub fn revoke(&self, grant_id: &str) {
        self.st().live.retain(|x| x.info.grant_id != grant_id);
        self.approvals.revoke_helper_grant(grant_id);
    }

    /// Lets a routed command through, or refuses it with `approval.grant_required`. Commands the policy does not list pass. A grant
    /// covers a command only while its signer still vouches for it ([`Self::recheck`]); when that cannot be told, it does not.
    pub fn gate(&self, c: &Cmd, now_ms: u64) -> Result<(), Error> {
        let name = command_name(c);
        if !self.policy.requires(name) {
            return Ok(());
        }
        let subject = subject(c);
        let project = subject.project.clone().or_else(|| {
            let agent = subject.agent.as_deref()?;
            let guard = self.project_of.lock().unwrap_or_else(|e| e.into_inner());
            guard.as_ref().and_then(|f| f(agent))
        });
        let generation = self.recheck(now_ms);
        let st = self.st();
        let covered = st.live.iter().any(|x| {
            let g = &x.info;
            generation.is_some_and(|at| !x.stale(at, now_ms))
                && g.expires_at_ms > now_ms
                && g.commands.iter().any(|p| matches(p, name))
                && match g.scope.split_once(':') {
                    None => g.scope == "node",
                    Some(("agent", id)) => subject.agent.as_deref() == Some(id),
                    // ADR-0115: a grant signed for a GitHub project's id in another case (before this node folded it) is that project's.
                    Some(("project", id)) => project
                        .as_deref()
                        .is_some_and(|p| agora_core::same_project(p, id)),
                    Some(_) => false,
                }
        });
        if covered {
            Ok(())
        } else {
            Err(Error::new(ErrorCode::ApprovalGrantRequired).with("command", name))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::approvals::Verifier;

    struct Fake;
    impl Verifier for Fake {
        fn verify(&self, device: &str, message: &[u8], signature: &[u8]) -> bool {
            device == "dev-1" && sign(message) == signature
        }
    }
    fn sign(message: &[u8]) -> Vec<u8> {
        let mut s = b"secret".to_vec();
        s.extend_from_slice(message);
        s
    }

    const NOW: u64 = 1_000_000;
    const HOUR: u64 = 3_600_000;

    fn grants(policy: &[&str]) -> Grants {
        let approvals = Arc::new(NodeApprovals::new("node-a", Box::new(Fake)));
        Grants::new(
            "node-a",
            approvals,
            SignedCommands(policy.iter().map(|s| (*s).to_string()).collect()),
        )
    }

    fn grant(id: &str, scope: &str, commands: &[&str], expires: u64) -> v1::GrantCreate {
        let commands: Vec<String> = commands.iter().map(|s| (*s).to_string()).collect();
        let signature = sign(&signed_message("node-a", id, scope, &commands, expires));
        v1::GrantCreate {
            grant_id: id.into(),
            scope: scope.into(),
            commands,
            expires_at_ms: expires,
            signer_device_id: "dev-1".into(),
            signature,
        }
    }

    fn prompt(agent: &str) -> Cmd {
        Cmd::AgentPrompt(v1::AgentPrompt {
            agent_id: agent.into(),
            text: "hi".into(),
        })
    }

    fn typing(agent: &str) -> Cmd {
        Cmd::TerminalInput(v1::TerminalInput {
            terminal_id: format!("t-{agent}"),
            data: b"ls\n".to_vec(),
        })
    }

    fn refused(r: Result<(), Error>) -> bool {
        matches!(r, Err(e) if e.code == ErrorCode::ApprovalGrantRequired)
    }

    #[test]
    fn the_signed_bytes_are_the_ones_the_web_client_builds() {
        // The same literal is asserted in apps/web/src/net/grant.test.ts: both sides must agree byte for byte.
        let m = signed_message(
            "laptop",
            "g1",
            "agent:a-1",
            &["agent.prompt".to_string(), "terminal.input".to_string()],
            1234,
        );
        assert_eq!(
            m,
            b"agora-grant-v1\0laptop\0g1\0agent:a-1\0agent.prompt,terminal.input\x001234".to_vec()
        );
    }

    #[test]
    fn without_the_policy_line_nothing_changes() {
        let g = grants(&[]);
        assert!(g.gate(&prompt("a-1"), NOW).is_ok());
        assert!(g.gate(&typing("a-1"), NOW).is_ok());
    }

    #[test]
    fn a_listed_command_without_a_grant_is_refused_and_unlisted_ones_pass() {
        let g = grants(&["agent.prompt", "terminal.*"]);
        assert!(refused(g.gate(&prompt("a-1"), NOW)));
        assert!(refused(g.gate(&typing("a-1"), NOW)));
        let list = Cmd::AgentList(v1::AgentList::default());
        assert!(g.gate(&list, NOW).is_ok(), "reading stays free");
    }

    #[test]
    fn a_valid_grant_allows_its_scope_and_commands_only() {
        let g = grants(&["agent.prompt", "terminal.input"]);
        g.create(
            &grant("g1", "agent:a-1", &["terminal.input"], NOW + HOUR),
            NOW,
        )
        .unwrap();
        assert!(g.gate(&typing("a-1"), NOW).is_ok());
        assert!(refused(g.gate(&typing("a-2"), NOW)), "another agent");
        assert!(refused(g.gate(&prompt("a-1"), NOW)), "another command");
        assert!(refused(g.gate(&typing("a-1"), NOW + HOUR)), "expired");
    }

    /// ADR-0108: the forget the hub passes on goes through the gate like any routed command: free unless the policy lists
    /// `agent.forget`, and then covered by a live grant for that agent (the gate names the agent the forget names) or its project.
    #[test]
    fn a_forget_the_hub_passes_on_needs_a_grant_for_its_agent_only_when_the_policy_lists_it() {
        let forget = |a: &str| Cmd::AgentForget(v1::AgentForget { agent_id: a.into() });
        assert!(
            grants(&[]).gate(&forget("a-1"), NOW).is_ok(),
            "unlisted: free"
        );
        let g = grants(&["agent.forget"]);
        assert!(refused(g.gate(&forget("a-1"), NOW)), "listed, no grant");
        g.create(
            &grant("g1", "agent:a-1", &["agent.forget"], NOW + HOUR),
            NOW,
        )
        .unwrap();
        assert!(g.gate(&forget("a-1"), NOW).is_ok());
        assert!(refused(g.gate(&forget("a-2"), NOW)), "another agent");
        g.set_project_resolver(Box::new(|a| (a == "a-3").then(|| "p-3".to_string())));
        g.create(&grant("g2", "project:p-3", &["agent.*"], NOW + HOUR), NOW)
            .unwrap();
        assert!(g.gate(&forget("a-3"), NOW).is_ok(), "its project's grant");
        assert!(refused(g.gate(&forget("a-4"), NOW)));
    }

    #[test]
    fn watching_a_terminal_or_a_chat_is_covered_by_a_grant_for_that_agent() {
        let g = grants(&["terminal.*", "chat.subscribe"]);
        g.set_project_resolver(Box::new(|a| (a == "a-3").then(|| "p-3".to_string())));
        let watch = |a: &str| {
            Cmd::TerminalSubscribe(v1::TerminalSubscribe {
                terminal_id: format!("t-{a}"),
                ..Default::default()
            })
        };
        let follow = |a: &str| Cmd::ChatSubscribe(v1::ChatSubscribe { agent_id: a.into() });
        assert!(refused(g.gate(&watch("a-1"), NOW)));
        assert!(refused(g.gate(&follow("a-1"), NOW)));
        g.create(
            &grant(
                "g1",
                "agent:a-1",
                &["terminal.*", "chat.subscribe"],
                NOW + HOUR,
            ),
            NOW,
        )
        .unwrap();
        assert!(g.gate(&watch("a-1"), NOW).is_ok());
        assert!(g.gate(&follow("a-1"), NOW).is_ok());
        assert!(refused(g.gate(&watch("a-2"), NOW)), "another agent");
        assert!(refused(g.gate(&follow("a-2"), NOW)), "another agent");
        // A project grant covers its agents.
        g.create(
            &grant("g2", "project:p-3", &["terminal.subscribe"], NOW + HOUR),
            NOW,
        )
        .unwrap();
        assert!(g.gate(&watch("a-3"), NOW).is_ok());
        assert!(refused(g.gate(&watch("a-2"), NOW)));
        // A machine-wide grant still covers every agent.
        g.create(
            &grant("g3", "node", &["terminal.subscribe"], NOW + HOUR),
            NOW,
        )
        .unwrap();
        assert!(g.gate(&watch("a-2"), NOW).is_ok());
        assert!(refused(g.gate(&follow("a-2"), NOW)), "another command");
    }

    #[test]
    fn a_node_grant_covers_every_agent_and_a_project_grant_its_agents() {
        let g = grants(&["agent.prompt", "agent.start"]);
        g.set_project_resolver(Box::new(|a| (a == "a-1").then(|| "p-1".to_string())));
        g.create(&grant("g1", "project:p-1", &["agent.*"], NOW + HOUR), NOW)
            .unwrap();
        assert!(g.gate(&prompt("a-1"), NOW).is_ok());
        assert!(refused(g.gate(&prompt("a-2"), NOW)));
        let start = |p: &str| {
            Cmd::AgentStart(v1::AgentStart {
                project_id: p.into(),
                ..Default::default()
            })
        };
        assert!(g.gate(&start("p-1"), NOW).is_ok());
        assert!(refused(g.gate(&start("p-2"), NOW)));
        g.create(&grant("g2", "node", &["agent.prompt"], NOW + HOUR), NOW)
            .unwrap();
        assert!(g.gate(&prompt("a-2"), NOW).is_ok());
    }

    /// ADR-0115: a project grant signed with a GitHub repository's id in its remote's case covers the project once this node folds the
    /// id, and the other way round; another repository or host does not match loosely.
    #[test]
    fn a_project_grant_in_another_case_of_a_github_id_covers_the_folded_project() {
        let root = "0123456789abcdef0123456789abcdef01234567";
        let start = |p: String| {
            Cmd::AgentStart(v1::AgentStart {
                project_id: p,
                ..Default::default()
            })
        };
        let g = grants(&["agent.start"]);
        g.create(
            &grant(
                "g1",
                &format!("project:github.com/O/R#{root}"),
                &["agent.*"],
                NOW + HOUR,
            ),
            NOW,
        )
        .unwrap();
        g.create(
            &grant(
                "g2",
                &format!("project:git.example.org/O/R#{root}"),
                &["agent.*"],
                NOW + HOUR,
            ),
            NOW,
        )
        .unwrap();
        assert!(g
            .gate(&start(format!("github.com/o/r#{root}")), NOW)
            .is_ok());
        assert!(g
            .gate(&start(format!("github.com/O/R#{root}")), NOW)
            .is_ok());
        assert!(refused(
            g.gate(&start(format!("github.com/o/r2#{root}")), NOW)
        ));
        assert!(refused(
            g.gate(&start(format!("git.example.org/o/r#{root}")), NOW)
        ));
    }

    #[test]
    fn a_bad_signature_wrong_node_or_wrong_signer_creates_nothing() {
        let g = grants(&["agent.prompt"]);
        let mut forged = grant("g1", "node", &["agent.prompt"], NOW + HOUR);
        forged.signature = b"nope".to_vec();
        assert!(g.create(&forged, NOW).is_err());
        // Signed for another node.
        let cmds = vec!["agent.prompt".to_string()];
        let mut other = grant("g2", "node", &["agent.prompt"], NOW + HOUR);
        other.signature = sign(&signed_message("node-b", "g2", "node", &cmds, NOW + HOUR));
        assert!(g.create(&other, NOW).is_err());
        // Changed after signing.
        let mut widened = grant("g3", "agent:a-1", &["agent.prompt"], NOW + HOUR);
        widened.scope = "node".into();
        assert!(g.create(&widened, NOW).is_err());
        let mut stranger = grant("g4", "node", &["agent.prompt"], NOW + HOUR);
        stranger.signer_device_id = "dev-2".into();
        assert!(g.create(&stranger, NOW).is_err());
        assert!(refused(g.gate(&prompt("a-1"), NOW)));
        assert!(g.list(NOW).is_empty());
    }

    #[test]
    fn expired_far_future_and_replayed_grants_are_refused() {
        let g = grants(&["agent.prompt"]);
        let e = g
            .create(&grant("g1", "node", &["agent.prompt"], NOW), NOW)
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::ApprovalExpired);
        assert!(g
            .create(
                &grant(
                    "g2",
                    "node",
                    &["agent.prompt"],
                    NOW + MAX_TTL_MS + 10 * SKEW_MS
                ),
                NOW
            )
            .is_err());
        g.create(&grant("g3", "node", &["agent.prompt"], NOW + HOUR), NOW)
            .unwrap();
        g.revoke("g3");
        assert!(
            g.create(&grant("g3", "node", &["agent.prompt"], NOW + HOUR), NOW)
                .is_err(),
            "a spent id is not accepted again, even after it was revoked"
        );
    }

    #[test]
    fn revoking_ends_the_grant_and_malformed_grants_are_refused() {
        let g = grants(&["agent.prompt"]);
        g.create(&grant("g1", "node", &["agent.prompt"], NOW + HOUR), NOW)
            .unwrap();
        assert_eq!(g.list(NOW).len(), 1);
        g.revoke("g1");
        g.revoke("never-existed");
        assert!(refused(g.gate(&prompt("a-1"), NOW)));
        for bad in [
            grant("g2", "everything", &["agent.prompt"], NOW + HOUR),
            grant("g3", "agent:", &["agent.prompt"], NOW + HOUR),
            grant("g4", "node", &[], NOW + HOUR),
            grant("", "node", &["agent.prompt"], NOW + HOUR),
            grant("g5", "node", &["a,b"], NOW + HOUR),
        ] {
            assert!(g.create(&bad, NOW).is_err());
        }
    }

    #[test]
    fn a_device_limited_to_some_projects_cannot_grant_beyond_them() {
        let limits = Arc::new(crate::limits::DeviceLimits::new(
            [("dev-1".to_string(), ["p-1".to_string()].into())].into(),
        ));
        limits.set_project_resolver(Box::new(|a| match a {
            "a-1" => Some("p-1".into()),
            "a-2" => Some("p-2".into()),
            _ => None,
        }));
        let approvals = Arc::new(NodeApprovals::new("node-a", Box::new(Fake)));
        let g = Grants::new(
            "node-a",
            approvals,
            SignedCommands(vec!["agent.prompt".into()]),
        )
        .with_limits(limits);
        let forbidden =
            |r: Result<(), Error>| matches!(r, Err(e) if e.code == ErrorCode::AuthForbidden);
        for scope in ["node", "project:p-2", "agent:a-2", "agent:a-3"] {
            assert!(
                forbidden(g.create(&grant("g", scope, &["agent.prompt"], NOW + HOUR), NOW)),
                "{scope}"
            );
        }
        assert!(g.list(NOW).is_empty());
        g.create(
            &grant("g-ok1", "project:p-1", &["agent.prompt"], NOW + HOUR),
            NOW,
        )
        .unwrap();
        g.create(
            &grant("g-ok2", "agent:a-1", &["agent.prompt"], NOW + HOUR),
            NOW,
        )
        .unwrap();
        assert_eq!(g.list(NOW).len(), 2);
    }

    /// Trusts exactly these devices, each in its term (signatures as [`sign`] makes them).
    struct Trusted(Vec<(&'static str, Term)>);
    impl Verifier for Trusted {
        fn verify(&self, device: &str, message: &[u8], signature: &[u8]) -> bool {
            self.0.iter().any(|(d, _)| *d == device) && sign(message) == signature
        }
        fn term(&self, device: &str) -> Option<Term> {
            self.0.iter().find(|(d, _)| *d == device).map(|(_, t)| *t)
        }
    }

    fn signed_by(device: &str, id: &str, scope: &str, cmd: &str) -> v1::GrantCreate {
        v1::GrantCreate {
            signer_device_id: device.into(),
            ..grant(id, scope, &[cmd], NOW + HOUR)
        }
    }

    const FOREVER: Term = Term {
        since: Some(1),
        until: None,
    };

    #[test]
    fn a_grant_ends_when_its_signer_is_no_longer_trusted() {
        // Audit F270: the device that signed a grant is revoked; the grant covers nothing from the next command on, and trusting
        // the device again does not bring it back. A grant from a device that stays trusted works unchanged.
        let approvals = Arc::new(NodeApprovals::new(
            "node-a",
            Box::new(Trusted(vec![("stolen", FOREVER), ("owner", FOREVER)])),
        ));
        let g = Grants::new(
            "node-a",
            Arc::clone(&approvals),
            SignedCommands(vec!["agent.prompt".into(), "terminal.input".into()]),
        );
        g.create(&signed_by("stolen", "g-s", "node", "agent.prompt"), NOW)
            .unwrap();
        g.create(
            &signed_by("owner", "g-o", "agent:a-1", "terminal.input"),
            NOW,
        )
        .unwrap();
        assert!(g.gate(&prompt("a-1"), NOW).is_ok());

        approvals.set_verifier(Box::new(Trusted(vec![("owner", FOREVER)])));
        assert!(refused(g.gate(&prompt("a-1"), NOW + 1)), "revoked signer");
        assert!(g.gate(&typing("a-1"), NOW + 1).is_ok(), "the other device");
        let live: Vec<String> = g.list(NOW + 1).into_iter().map(|x| x.grant_id).collect();
        assert_eq!(live, ["g-o"]);

        approvals.set_verifier(Box::new(Trusted(vec![
            ("stolen", FOREVER),
            ("owner", FOREVER),
        ])));
        assert!(
            refused(g.gate(&prompt("a-1"), NOW + 2)),
            "trusted again: the old grant stays ended"
        );
        assert!(g.gate(&typing("a-1"), NOW + 2).is_ok());
    }

    #[test]
    fn a_grant_ends_with_its_signers_term() {
        let term = Term {
            since: Some(1),
            until: Some(NOW + 100),
        };
        let approvals = Arc::new(NodeApprovals::new(
            "node-a",
            Box::new(Trusted(vec![("endorsed", term), ("owner", FOREVER)])),
        ));
        let g = Grants::new(
            "node-a",
            Arc::clone(&approvals),
            SignedCommands(vec!["agent.prompt".into()]),
        );
        // An endorsed device's trust ends before its grant does: covered up to the last millisecond, then never again.
        g.create(&signed_by("endorsed", "g-e", "node", "agent.prompt"), NOW)
            .unwrap();
        assert!(g.gate(&prompt("a-1"), NOW + 99).is_ok());
        assert!(refused(g.gate(&prompt("a-1"), NOW + 100)));
        assert!(refused(g.gate(&prompt("a-1"), NOW + 99)), "stays ended");

        // The same device trusted afresh (removed and trusted again, unseen by this node): a new term, the old grant does not carry.
        g.create(&signed_by("owner", "g-o", "node", "agent.prompt"), NOW)
            .unwrap();
        approvals.set_verifier(Box::new(Trusted(vec![(
            "owner",
            Term {
                since: Some(2),
                until: None,
            },
        )])));
        assert!(refused(g.gate(&prompt("a-1"), NOW + 1)), "a later term");
    }

    #[test]
    fn a_grant_ends_when_its_signers_limits_no_longer_allow_its_scope() {
        // ADR-0061: a device endorsed by a limited device inherits its limit through the trusted-devices file (`projects=`).
        let dir = std::env::temp_dir().join(format!("agora-grants-f270-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(crate::approvals::TRUSTED_DEVICES_FILE);
        let key = format!("04{}", "11".repeat(64));
        let line = |projects: &str| {
            format!("owner {key}\nmember {key} since=1 expires=99999999999999 via=owner projects={projects}\n")
        };
        std::fs::write(&file, line("p-1")).unwrap();
        let limits = Arc::new(crate::limits::DeviceLimits::default());
        limits.follow_trust_file(&file);
        let trusted = || Box::new(Trusted(vec![("member", FOREVER), ("owner", FOREVER)]));
        let approvals = Arc::new(NodeApprovals::new("node-a", trusted()));
        let g = Grants::new(
            "node-a",
            Arc::clone(&approvals),
            SignedCommands(vec!["agent.start".into()]),
        )
        .with_limits(limits);
        let start = |p: &str| {
            Cmd::AgentStart(v1::AgentStart {
                project_id: p.into(),
                ..Default::default()
            })
        };
        g.create(
            &signed_by("member", "g-m", "project:p-1", "agent.start"),
            NOW,
        )
        .unwrap();
        assert!(g.gate(&start("p-1"), NOW).is_ok());

        // Narrowed to p-2. The node reads the changed file again (here: the verifier replaced, as the reload does).
        std::fs::write(&file, line("p-2")).unwrap();
        approvals.set_verifier(trusted());
        assert!(refused(g.gate(&start("p-1"), NOW)));
        // Widened back: the grant stays ended.
        std::fs::write(&file, line("p-1")).unwrap();
        approvals.set_verifier(trusted());
        assert!(refused(g.gate(&start("p-1"), NOW)));
        assert!(g.list(NOW).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_team_default_covers_every_command_that_changes_something_and_no_reading() {
        let d = SignedCommands::team_default();
        for name in [
            "agent.start",
            "agent.stop",
            "agent.prompt",
            "agent.set_mode",
            "agent.set_model",
            "agent.set_config",
            "agent.cancel_turn",
            "task.dispatch",
            "terminal.input",
            "files.write",
            "worktree.remove",
        ] {
            assert!(d.requires(name), "{name}");
        }
        for name in [
            "agent.list",
            "files.read",
            "git.diff",
            "terminal.subscribe",
            "project.list",
            "usage.report",
        ] {
            assert!(!d.requires(name), "{name}");
        }
    }
}
