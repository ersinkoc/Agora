//! User-signed grants for the commands a client sends through the hub (ADR-0041).
//!
//! A node whose policy has a `signed_commands` line refuses those commands when they arrive routed through the hub, unless
//! a live grant covers them. A grant is signed by a trusted device (the devices of ADR-0028) and verified here, so a hub
//! that forges one fails the check. Grants live in memory only: a restart ends them on purpose. Commands that arrive on the
//! node's own control socket never pass through here (the person at the machine needs no grant).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use agora_protocol::v1::{self, command_request::Command as Cmd};
use agora_protocol::{Error, ErrorCode};

use crate::approvals::NodeApprovals;
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
    live: Vec<v1::GrantInfo>,
    spent: VecDeque<String>,
}

type Resolver = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

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
}

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
        Cmd::TerminalInput(t) => {
            s.agent = agent_of_terminal(&t.terminal_id).map(|a| a.as_str().to_string());
        }
        Cmd::TerminalResize(t) => {
            s.agent = agent_of_terminal(&t.terminal_id).map(|a| a.as_str().to_string());
        }
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
        }
    }

    pub fn with_directory_nonce_file(mut self, path: std::path::PathBuf) -> Self {
        self.directory_nonce_file = Some(path);
        self
    }

    /// Directory actions are always exact-path, short-lived, owner-signed and single-use on the routed path.
    pub fn directory_action(&self, c: &Cmd, now: u64) -> Result<(), Error> {
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
            _ => return Ok(()),
        };
        let bad =
            || Error::new(ErrorCode::ApprovalSignatureInvalid).with("approval_id", "directory");
        let a = auth.ok_or_else(bad)?;
        if [path, kind, project, a.nonce.as_str(), a.device_id.as_str()]
            .iter()
            .any(|s| s.contains('\0') || s.len() > 4096)
            || a.nonce.len() != 32
            || !a.nonce.bytes().all(|b| b.is_ascii_hexdigit())
            || a.expires_at_ms <= now
            || a.expires_at_ms > now.saturating_add(5 * 60 * 1000)
        {
            return Err(bad());
        }
        if !self.limits.allows_scope(&a.device_id, "node") {
            return Err(Error::new(ErrorCode::AuthForbidden).with("action", "directory"));
        }
        let msg = crate::directory::signed_message(
            &self.node_id,
            op,
            path,
            kind,
            project,
            option,
            identity_change,
            &a.nonce,
            a.expires_at_ms,
        );
        if !self
            .approvals
            .verify_device(&a.device_id, &msg, &a.signature)
        {
            return Err(bad());
        }
        let key = format!("{}:{}", a.device_id, a.nonce);
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
        used.insert(key, a.expires_at_ms);
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
        if !self
            .approvals
            .verify_device(&g.signer_device_id, &message, &g.signature)
        {
            return Err(bad());
        }
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
        st.live.retain(|x| x.expires_at_ms > now_ms);
        if st.spent.iter().any(|s| *s == g.grant_id) || st.live.len() >= MAX_LIVE {
            return Err(bad());
        }
        st.spent.push_back(g.grant_id.clone());
        while st.spent.len() > SPENT_MEMORY {
            st.spent.pop_front();
        }
        st.live.push(v1::GrantInfo {
            grant_id: g.grant_id.clone(),
            scope: g.scope.clone(),
            commands: g.commands.clone(),
            expires_at_ms: g.expires_at_ms,
            signer_device_id: g.signer_device_id.clone(),
        });
        Ok(())
    }

    pub fn list(&self, now_ms: u64) -> Vec<v1::GrantInfo> {
        let mut st = self.st();
        st.live.retain(|x| x.expires_at_ms > now_ms);
        st.live.clone()
    }

    /// Ends a grant. An id that is not live is not an error (ending access is always allowed and idempotent).
    pub fn revoke(&self, grant_id: &str) {
        self.st().live.retain(|x| x.grant_id != grant_id);
    }

    /// Lets a routed command through, or refuses it with `approval.grant_required`. Commands the policy does not list pass.
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
        let st = self.st();
        let covered = st.live.iter().any(|g| {
            g.expires_at_ms > now_ms
                && g.commands.iter().any(|p| matches(p, name))
                && match g.scope.split_once(':') {
                    None => g.scope == "node",
                    Some(("agent", id)) => subject.agent.as_deref() == Some(id),
                    Some(("project", id)) => project.as_deref() == Some(id),
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

    #[test]
    fn the_team_default_covers_every_command_that_changes_something_and_no_reading() {
        let d = SignedCommands::team_default();
        for name in [
            "agent.start",
            "agent.stop",
            "agent.prompt",
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
