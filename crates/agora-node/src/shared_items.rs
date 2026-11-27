//! ADR-0122 step 3: the shared skills and MCP servers this node gives its agents. The node is the authority, the hub only the first
//! gate: every word the hub hands over is verified here against this node's own trusted devices (ADR-0061, a device limited to some
//! projects never counts), its content hash and the rules in `agora_protocol::share`, and the newest `issued_ms` this node ever saw for
//! the name (kept on disk, so a replayed older word is refused even after a restart); then this node's policy decides
//! (`share.items`, `share.mcp_commands`). The hub can only narrow: an item it stops listing is dropped.
//!
//! The hub is asked for the list (names and hashes, no content) when the link is set and every [`POLL`]; a changed hash is fetched
//! with `ShareItemGet`. Each refusal is said once per version with `ShareItemRefused`. What a launch gets is [`SharedItems::for_launch`]
//! (step 3b injects it).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
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
    /// ADR-0122 Amendment A: the secret names a shared MCP server may receive on this machine (`share.secrets`, default none). A
    /// server that names any other is not given here, whatever the owner signed.
    pub secrets: BTreeSet<String>,
}

impl Default for SharePolicy {
    fn default() -> Self {
        Self {
            on: true,
            mcp_commands: None,
            mcp_urls: true,
            secrets: BTreeSet::new(),
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
            if item.name != w.name
                || check_item(item).is_err()
                || agora_protocol::share::check_item_shown(item, agora_core::text::is_hidden)
                    .is_err()
            {
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
        // A loader's or an interpreter's own variable would make an allowed program (npx, uvx) run anything: never from an item.
        let env_loads_code = w
            .item
            .as_ref()
            .and_then(|i| i.mcp.as_ref())
            .is_some_and(|m| m.env.iter().any(|v| loads_code(&v.name)));
        if env_loads_code {
            return Err(refusal("share.refused", "env_not_allowed"));
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
    /// ADR-0122 Amendment A: the launches still to be asked for, by launch id: (agent, item). Each id is spent by its first ask.
    launches: Mutex<BTreeMap<String, Launch>>,
    /// Reads one of this node's secrets (ADR-0084's store); without it no server that names a secret is given.
    secret: Mutex<Option<SecretReader>>,
    /// The program the agent's tool starts for a server that names a secret: this node's own binary (`mcp-launch`).
    launcher: PathBuf,
}

/// What one launch id was given for: the agent, the item and its version, and the project and kind the agent runs in.
#[derive(Debug, Clone)]
struct Launch {
    agent: String,
    item: String,
    item_hash: String,
    project: String,
    kind: String,
    /// ADR-0133: the one instance of this binding that may be live, if any.
    instance: Option<Instance>,
    /// When the last instance ended badly: the next start waits [`RETRY_AFTER`].
    failed_at: Option<Instant>,
    /// When instances were started in the last minute (at most [`STARTS_PER_MINUTE`]).
    starts: VecDeque<Instant>,
}

/// ADR-0133: a server instance the launcher started (or is starting) for a binding.
#[derive(Debug, Clone)]
struct Instance {
    /// The server's process, as the launcher reported it (where this platform can tell a process again: not on Windows, where the
    /// launcher binds its server to itself, so the launcher's end is the server's).
    process: Option<agora_platform::ProcessIdentity>,
    /// The launcher's connection went away without saying the server ended.
    dropped: bool,
}

/// ADR-0133: the least time after an instance ended badly before the next may start, and the most starts a minute per binding.
pub const RETRY_AFTER: Duration = Duration::from_secs(5);
pub const STARTS_PER_MINUTE: usize = 6;

/// Reads a secret by name from the node's store.
pub type SecretReader = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// ADR-0122 step 5: whether a skill asks for nothing but to be read. Its `SKILL.md` must start with a header (`---` lines) whose keys
/// are only `name` and `description` (comments, blank lines and indented continuation lines aside): a tool may read more keys from
/// it (the tools allowed without asking, commands to run), which would let a signed skill go past this machine's own policy.
pub fn skill_header_ok(skill: &v1::SkillDef) -> bool {
    let Some(file) = skill.files.iter().find(|f| f.path == "SKILL.md") else {
        return false;
    };
    let Ok(text) = std::str::from_utf8(&file.content) else {
        return false;
    };
    let mut lines = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l));
    if lines.next() != Some("---") {
        return false;
    }
    // The first key starts the line, which sets the mapping's indentation at 0: an indented line after it can only continue the value
    // above it. An indented line before any key (a whole mapping indented, which YAML allows) is refused.
    let mut keyed = false;
    for line in lines {
        if line == "---" {
            return keyed;
        }
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if !keyed {
                return false;
            }
            continue;
        }
        keyed = true;
        match line.split_once(':') {
            Some((key, _)) if key == "name" || key == "description" => {}
            _ => return false,
        }
    }
    false
}

/// ADR-0122 step 5: writes `skills` into `dir` laid out as `layout` says, replacing what was there. The names and paths were checked
/// with the item (`agora_protocol::share`) and are checked again here, so nothing lands outside `dir`.
pub fn write_skills(
    dir: &Path,
    layout: agora_adapters::manifest::SkillsLayout,
    skills: &[(String, v1::SkillDef)],
) -> std::io::Result<()> {
    let bad = || std::io::Error::from(std::io::ErrorKind::InvalidInput);
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    match layout {
        agora_adapters::manifest::SkillsLayout::ClaudePlugin => {
            let meta = dir.join(".claude-plugin");
            std::fs::create_dir_all(&meta)?;
            agora_platform::write_private(&meta.join("plugin.json"), br#"{"name":"shared"}"#)?;
            for (name, skill) in skills {
                if !agora_protocol::share::valid_name(name, agora_protocol::share::MAX_NAME) {
                    return Err(bad());
                }
                let home = dir.join("skills").join(name);
                for f in &skill.files {
                    if !agora_protocol::share::valid_skill_path(&f.path) {
                        return Err(bad());
                    }
                    let path = home.join(&f.path);
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    agora_platform::write_private(&path, &f.content)?;
                }
            }
        }
    }
    Ok(())
}

/// The names of the secrets a value refers to (`{secret:NAME}`), in order, each once.
pub fn secret_names(value: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = value;
    while let Some(at) = rest.find("{secret:") {
        let after = &rest[at + "{secret:".len()..];
        let Some(end) = after.find('}') else { break };
        let name = after[..end].to_string();
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
        rest = &after[end + 1..];
    }
    out
}

impl crate::mesh_server::SharedLaunch for SharedItems {
    fn launch(
        &self,
        agent: &agora_core::AgentId,
        item: &str,
        launch_id: &str,
    ) -> Result<crate::mesh_server::LaunchPlan, &'static str> {
        SharedItems::launch(self, agent.as_str(), item, launch_id)
    }
    fn started(&self, agent: &agora_core::AgentId, item: &str, launch_id: &str, pid: u32) {
        self.instance_started(agent.as_str(), item, launch_id, pid);
    }
    fn ended(&self, agent: &agora_core::AgentId, item: &str, launch_id: &str, clean: bool) {
        self.instance_ended(agent.as_str(), item, launch_id, clean);
    }
    fn dropped(&self, agent: &agora_core::AgentId, item: &str, launch_id: &str) {
        self.instance_dropped(agent.as_str(), item, launch_id);
    }
    fn revoked(&self, agent: &agora_core::AgentId, item: &str, launch_id: &str) -> bool {
        self.instance_revoked(agent.as_str(), item, launch_id)
    }
}

/// Environment names a shared server may not set (review of b9dfb5fe): what a dynamic loader or an interpreter reads before the
/// program's own code, and where programs are looked up. Compared without case (Windows' environment ignores it).
fn loads_code(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    const EXACT: &[&str] = &[
        "GCONV_PATH",
        "PERL5DB",
        "PYTHONEXECUTABLE",
        "BROWSER",
        "PATH",
        "PATHEXT",
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "LD_AUDIT",
        "NODE_OPTIONS",
        "NODE_PATH",
        "PYTHONPATH",
        "PYTHONSTARTUP",
        "PYTHONHOME",
        "PYTHONUSERBASE",
        "RUBYOPT",
        "RUBYLIB",
        "PERL5OPT",
        "PERL5LIB",
        "PERLLIB",
        "BASH_ENV",
        "ENV",
        "ZDOTDIR",
        "JAVA_TOOL_OPTIONS",
        "_JAVA_OPTIONS",
        "JDK_JAVA_OPTIONS",
        "CLASSPATH",
        "DENO_DIR",
        "BUN_INSTALL",
        "COMSPEC",
        "PSMODULEPATH",
        "GIT_SSH",
        "GIT_SSH_COMMAND",
        "GIT_EXEC_PATH",
        "NPM_CONFIG_PREFIX",
        "NPM_CONFIG_USERCONFIG",
        "UV_PYTHON",
        "LUA_PATH",
        "LUA_CPATH",
    ];
    EXACT.contains(&n.as_str())
        || n.starts_with("DYLD_")
        || n.starts_with("LD_")
        || n.starts_with("NPM_CONFIG_")
}

/// A launch id: 32 random bytes as hex; `None` when the system gives no randomness (the server is then not given).
fn launch_id() -> Option<String> {
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).ok()?;
    Some(b.iter().map(|x| format!("{x:02x}")).collect())
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
            launches: Mutex::new(BTreeMap::new()),
            secret: Mutex::new(None),
            launcher: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("agora")),
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
    /// trust and policy. A server that names a secret (`{secret:NAME}` in its environment or headers) is not given yet and is said
    /// once (`secret_needs_launcher`): a secret must never sit in a file the agent can read, since whatever an agent reads may go to
    /// its model's provider (agora-ac's review of afafe8f3); a node launcher that puts it only into the server's own environment
    /// comes next. What was given is kept for the agent's `AgentStarted`.
    pub fn servers_for(
        &self,
        agent: &str,
        project: &str,
        kind: &str,
        socket: &Path,
    ) -> Vec<agora_adapters::inject::SharedServer> {
        self.rejudge();
        let mut servers = Vec::new();
        let mut given = Vec::new();
        for w in self.for_launch(project, kind) {
            let Some(m) = w.item.as_ref().and_then(|i| i.mcp.clone()) else {
                continue;
            };
            // ADR-0122 Amendment A: a program whose environment names secrets is given through the node's launcher, only when
            // this machine lets every one of them go to shared servers and holds it; a server at a URL whose headers name one
            // waits for a later step. Each refusal is said once per version.
            let header_secrets = m.headers.iter().any(|v| v.value.contains("{secret:"));
            let names: Vec<String> = m.env.iter().flat_map(|v| secret_names(&v.value)).collect();
            if header_secrets || !names.is_empty() {
                let refusal = if header_secrets {
                    Some(("share.refused", "secret_needs_launcher".to_string()))
                } else {
                    let st = self.st();
                    let allowed = &st.policy.secrets;
                    let reader = self.secret.lock().unwrap_or_else(|e| e.into_inner());
                    names.iter().find_map(|n| {
                        if !allowed.contains(n) {
                            Some(("share.refused", "secret_not_allowed".to_string()))
                        } else if reader.as_ref().and_then(|r| r(n)).is_none() {
                            Some(("share.secret_missing", n.clone()))
                        } else {
                            None
                        }
                    })
                };
                if let Some((code, reason)) = refusal {
                    let mut st = self.st();
                    if st
                        .refused
                        .insert((w.name.clone(), format!("{}#{reason}", w.item_hash)))
                    {
                        self.out.lock().unwrap_or_else(|e| e.into_inner()).push(
                            Body::ShareItemRefused(v1::ShareItemRefused {
                                node_id: self.node_id.clone(),
                                name: w.name.clone(),
                                item_hash: w.item_hash.clone(),
                                code: code.to_string(),
                                reason,
                            }),
                        );
                    }
                    continue;
                }
                let Some(id) = launch_id() else { continue };
                {
                    // Only the id in the agent's current file is live: an earlier launch's id for the same item goes.
                    let mut launches = self.launches.lock().unwrap_or_else(|e| e.into_inner());
                    launches.retain(|_, l| !(l.agent == agent && l.item == w.name));
                    launches.insert(
                        id.clone(),
                        Launch {
                            agent: agent.to_string(),
                            item: w.name.clone(),
                            item_hash: w.item_hash.clone(),
                            project: project.to_string(),
                            kind: kind.to_string(),
                            instance: None,
                            failed_at: None,
                            starts: VecDeque::new(),
                        },
                    );
                }
                given.push(v1::SharedItemRef {
                    name: w.name.clone(),
                    item_hash: w.item_hash.clone(),
                });
                servers.push(agora_adapters::inject::SharedServer {
                    name: w.name.clone(),
                    command: self.launcher.to_string_lossy().into_owned(),
                    args: vec![
                        "mcp-launch".into(),
                        "--socket".into(),
                        socket.to_string_lossy().into_owned(),
                        w.name.clone(),
                        id,
                    ],
                    env: Vec::new(),
                    url: String::new(),
                    headers: Vec::new(),
                });
                continue;
            }
            let pairs = |v: &[v1::ShareValue]| {
                v.iter()
                    .map(|x| (x.name.clone(), x.value.clone()))
                    .collect()
            };
            given.push(v1::SharedItemRef {
                name: w.name.clone(),
                item_hash: w.item_hash.clone(),
            });
            servers.push(agora_adapters::inject::SharedServer {
                name: w.name.clone(),
                command: m.command.clone(),
                args: m.args.clone(),
                env: pairs(&m.env),
                url: m.url.clone(),
                headers: pairs(&m.headers),
            });
        }
        self.given
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(agent.to_string(), given);
        servers
    }

    /// ADR-0122 step 5: the shared skills an agent of `kind` in `project` gets at its launch, judged again first, written into
    /// [`Self::skills_dir`] laid out as `layout` says. A skill whose `SKILL.md` header asks for more than its name and description
    /// ([`skill_header_ok`]) is left out, and a folder that cannot be written gives no skill: the agent starts without them, and
    /// each is said once per version. What was written is added to what the agent was given (call after [`Self::servers_for`], or
    /// after [`Self::forget_given`]).
    pub fn skills_for(
        &self,
        agent: &str,
        project: &str,
        kind: &str,
        layout: agora_adapters::manifest::SkillsLayout,
    ) -> Vec<(String, v1::SkillDef)> {
        self.rejudge();
        let dir = self.skills_dir(agent);
        let mut skills = Vec::new();
        let mut words = Vec::new();
        for w in self.for_launch(project, kind) {
            let Some(sk) = w.item.as_ref().and_then(|i| i.skill.clone()) else {
                continue;
            };
            if !skill_header_ok(&sk) {
                self.say_refused(&w, "share.refused", "skill_header");
                continue;
            }
            skills.push((w.name.clone(), sk));
            words.push(w);
        }
        if skills.is_empty() {
            let _ = std::fs::remove_dir_all(&dir);
            return skills;
        }
        if let Err(e) = write_skills(&dir, layout, &skills) {
            crate::log_line!("shared skills: not written for {agent}: {}", e.kind());
            let _ = std::fs::remove_dir_all(&dir);
            for w in &words {
                self.say_refused(w, "share.refused", "skill_not_written");
            }
            return Vec::new();
        }
        self.given
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(agent.to_string())
            .or_default()
            .extend(words.iter().map(|w| v1::SharedItemRef {
                name: w.name.clone(),
                item_hash: w.item_hash.clone(),
            }));
        skills
    }

    /// Says once per version and reason that this node does not give `w` to its agents.
    fn say_refused(&self, w: &ShareItemSigned, code: &str, reason: &str) {
        let mut st = self.st();
        if st
            .refused
            .insert((w.name.clone(), format!("{}#{reason}", w.item_hash)))
        {
            self.out
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(Body::ShareItemRefused(v1::ShareItemRefused {
                    node_id: self.node_id.clone(),
                    name: w.name.clone(),
                    item_hash: w.item_hash.clone(),
                    code: code.to_string(),
                    reason: reason.to_string(),
                }));
        }
    }

    /// What `agent` was given is forgotten, before a launch that gives it no server.
    pub fn forget_given(&self, agent: &str) {
        self.given
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(agent);
    }

    /// The folder of the shared skills an agent was given (outside its worktree; the node's own).
    pub fn skills_dir(&self, agent: &str) -> PathBuf {
        self.dir.join("skills").join(agent)
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

    /// ADR-0122 Amendment A: where this node's secrets are read from (its store, ADR-0084).
    pub fn set_secrets(&self, reader: SecretReader) {
        *self.secret.lock().unwrap_or_else(|e| e.into_inner()) = Some(reader);
    }

    /// ADR-0122 Amendment A: what the launcher of `item` for `agent` starts, for the launch id it was given at the agent's launch.
    /// The id is spent by this ask, whatever the answer; the item is judged again with today's trust and policy, and its secrets
    /// are read now (`unknown_launch`, `unknown_item`, `secret_not_allowed`, `secret_missing`).
    pub fn launch(
        &self,
        agent: &str,
        item: &str,
        launch_id: &str,
    ) -> Result<crate::mesh_server::LaunchPlan, &'static str> {
        let now = Instant::now();
        // ADR-0133: the binding stays for the agent's run; one instance at a time, a pause after a bad end, a budget per minute. The
        // instance is reserved here, so a second ask while this one is answered is refused, and released if this one is refused.
        let given = {
            let mut launches = self.launches.lock().unwrap_or_else(|e| e.into_inner());
            let Some(l) = launches.get_mut(launch_id) else {
                return Err("unknown_launch");
            };
            if l.agent != agent || l.item != item {
                return Err("unknown_launch");
            }
            if let Some(inst) = &l.instance {
                let alive = match &inst.process {
                    Some(p) => agora_platform::process_still_running(p),
                    None => !inst.dropped,
                };
                if alive {
                    crate::log_line!("shared launch: {agent} {item} refused: instance_live");
                    return Err("instance_live");
                }
                l.instance = None;
            }
            if l.failed_at
                .is_some_and(|t| now.saturating_duration_since(t) < RETRY_AFTER)
            {
                return Err("too_soon");
            }
            while l
                .starts
                .front()
                .is_some_and(|t| now.saturating_duration_since(*t) >= Duration::from_secs(60))
            {
                l.starts.pop_front();
            }
            if l.starts.len() >= STARTS_PER_MINUTE {
                crate::log_line!("shared launch: {agent} {item} refused: start_budget");
                return Err("start_budget");
            }
            l.instance = Some(Instance {
                process: None,
                dropped: false,
            });
            l.starts.push_back(now);
            (l.item_hash.clone(), l.project.clone(), l.kind.clone())
        };
        let answer = self.plan_for(item, &given);
        let mut launches = self.launches.lock().unwrap_or_else(|e| e.into_inner());
        match &answer {
            Ok(_) => crate::log_line!("shared launch: {agent} {item} started"),
            Err(reason) => {
                crate::log_line!("shared launch: {agent} {item} refused: {reason}");
                if let Some(l) = launches.get_mut(launch_id) {
                    l.instance = None;
                    l.starts.pop_back();
                }
            }
        }
        answer
    }

    /// The program of `item` with its secrets filled, judged again with today's trust and policy, for the version, project and kind
    /// the binding `(item hash, project, kind)` was made for.
    fn plan_for(
        &self,
        item: &str,
        given: &(String, String, String),
    ) -> Result<crate::mesh_server::LaunchPlan, &'static str> {
        struct Given<'a> {
            item_hash: &'a str,
            project: &'a str,
            kind: &'a str,
        }
        let given = Given {
            item_hash: &given.0,
            project: &given.1,
            kind: &given.2,
        };
        self.rejudge();
        let (m, allowed) = {
            let st = self.st();
            if !st.policy.on {
                return Err("unknown_item");
            }
            // The version the agent was given, and a scope that still covers it (the owner may have changed or narrowed it since).
            let w = st.used.get(item).ok_or("unknown_item")?;
            if w.item_hash != given.item_hash
                || !covers(
                    &w.scope.clone().unwrap_or_default(),
                    given.project,
                    given.kind,
                )
            {
                return Err("unknown_item");
            }
            let m = w
                .item
                .as_ref()
                .and_then(|i| i.mcp.clone())
                .ok_or("unknown_item")?;
            (m, st.policy.secrets.clone())
        };
        if m.command.is_empty() {
            return Err("unknown_item");
        }
        let reader = self.secret.lock().unwrap_or_else(|e| e.into_inner());
        let Some(read) = reader.as_ref() else {
            return Err("secret_missing");
        };
        let mut env = Vec::with_capacity(m.env.len());
        for v in &m.env {
            if secret_names(&v.value).iter().any(|n| !allowed.contains(n)) {
                return Err("secret_not_allowed");
            }
            let filled = fill_secrets(&v.value, &|n| read(n)).map_err(|_| "secret_missing")?;
            env.push((v.name.clone(), filled));
        }
        Ok(crate::mesh_server::LaunchPlan {
            command: m.command,
            args: m.args,
            env,
        })
    }

    /// ADR-0133: the launcher started the server of the binding `launch_id` as process `pid`.
    pub fn instance_started(&self, agent: &str, item: &str, launch_id: &str, pid: u32) {
        let mut launches = self.launches.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(inst) = launches
            .get_mut(launch_id)
            .filter(|l| l.agent == agent && l.item == item)
            .and_then(|l| l.instance.as_mut())
        {
            inst.process = agora_platform::process_identity(pid).ok();
        }
    }

    /// ADR-0133: the launcher said the server ended (`clean`: it exited with 0); a bad end makes the next start wait.
    pub fn instance_ended(&self, agent: &str, item: &str, launch_id: &str, clean: bool) {
        let mut launches = self.launches.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(l) = launches
            .get_mut(launch_id)
            .filter(|l| l.agent == agent && l.item == item)
        {
            l.instance = None;
            if !clean {
                l.failed_at = Some(Instant::now());
            }
            crate::log_line!(
                "shared launch: {agent} {item} ended{}",
                if clean { "" } else { " badly" }
            );
        }
    }

    /// ADR-0133: the launcher's connection went away without saying the server ended. A server whose process is known and still runs
    /// keeps the binding taken until it ends; otherwise the instance is over, as a bad end.
    pub fn instance_dropped(&self, agent: &str, item: &str, launch_id: &str) {
        let mut launches = self.launches.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(l) = launches
            .get_mut(launch_id)
            .filter(|l| l.agent == agent && l.item == item)
        {
            match l.instance.as_mut() {
                Some(inst)
                    if inst
                        .process
                        .as_ref()
                        .is_some_and(agora_platform::process_still_running) =>
                {
                    inst.dropped = true;
                }
                Some(_) => {
                    l.instance = None;
                    l.failed_at = Some(Instant::now());
                }
                None => {}
            }
            crate::log_line!("shared launch: {agent} {item} launcher gone");
        }
    }

    /// ADR-0133: whether the launcher should close its server now: the binding is gone (the agent ended), or the item it was made for
    /// is no longer in use here as that version and scope (removed, replaced, its signer untrusted, the policy changed).
    pub fn instance_revoked(&self, agent: &str, item: &str, launch_id: &str) -> bool {
        let binding = {
            let launches = self.launches.lock().unwrap_or_else(|e| e.into_inner());
            match launches.get(launch_id) {
                Some(l) if l.agent == agent && l.item == item => {
                    (l.item_hash.clone(), l.project.clone(), l.kind.clone())
                }
                _ => return true,
            }
        };
        let st = self.st();
        !st.policy.on
            || !st.used.get(item).is_some_and(|w| {
                w.item_hash == binding.0
                    && covers(&w.scope.clone().unwrap_or_default(), &binding.1, &binding.2)
            })
    }

    /// The agent ended: its file of shared servers is removed, what it was given forgotten, and its launch ids spent.
    pub fn agent_ended(&self, agent: &str) {
        self.launches
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, l| l.agent != agent);
        let _ = std::fs::remove_file(self.agent_file(agent));
        let _ = std::fs::remove_dir_all(self.skills_dir(agent));
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
        // Signed as it is, but with a direction override in its program: what the owner saw is not what would run.
        let unseen = word(
            "db",
            Some(item("db", "np\u{202e}x", "")),
            12,
            ShareScope::default(),
        );
        assert_eq!(
            judge(&unseen, None, &on, trusted).unwrap_err().reason,
            "invalid"
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

    /// At launch an agent gets the servers in its scope; one that names a secret is not given (never into a file the agent can read)
    /// and is said once; what it got is kept for its start, and forgotten when it ends.
    #[test]
    fn a_launch_gets_its_servers_and_one_naming_a_secret_waits_for_the_launcher() {
        let d = temp("launch");
        let s = open(&d, SharePolicy::default());
        let mut db = item("db", "npx", "");
        db.mcp.as_mut().unwrap().env = vec![v1::ShareValue {
            name: "MODE".into(),
            value: "readonly".into(),
        }];
        let mut docs = item("docs", "", "https://mcp.example.com");
        docs.mcp.as_mut().unwrap().headers = vec![v1::ShareValue {
            name: "Authorization".into(),
            value: "Bearer {secret:DOCS}".into(),
        }];
        s.take(word("db", Some(db), 1, ShareScope::default()));
        s.take(word("docs", Some(docs), 1, ShareScope::default()));
        let got = s.servers_for(
            "agent-1",
            "p",
            "claude-code",
            Path::new("/run/m/agent-1.sock"),
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].env, [("MODE".to_string(), "readonly".to_string())]);
        assert_eq!(
            s.given("agent-1")
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>(),
            ["db"]
        );
        s.servers_for(
            "agent-2",
            "p",
            "claude-code",
            Path::new("/run/m/agent-2.sock"),
        );
        let said = s.take_events();
        assert!(
            matches!(&said[..], [Body::ShareItemRefused(r)] if r.reason == "secret_needs_launcher" && r.name == "docs"),
            "once: {said:?}"
        );
        s.agent_ended("agent-1");
        assert!(s.given("agent-1").is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// ADR-0122 step 5: a launch gets the skills in its scope beside its servers, both named in what it was given; they are written as
    /// a plugin folder of the node's own, a path that would leave it is refused, and the folder goes when the agent ends.
    #[test]
    fn a_launch_gets_its_skills_written_as_the_kind_loads_them() {
        let d = temp("skills");
        let s = open(&d, SharePolicy::default());
        let layout = agora_adapters::manifest::SkillsLayout::ClaudePlugin;
        let skill = |name: &str, files: &[(&str, &str)]| ShareItem {
            name: name.into(),
            kind: ShareItemKind::Skill as i32,
            mcp: None,
            skill: Some(v1::SkillDef {
                description: "the review rules".into(),
                files: files
                    .iter()
                    .map(|(p, c)| v1::SkillFile {
                        path: (*p).into(),
                        content: c.as_bytes().to_vec(),
                    })
                    .collect(),
            }),
        };
        let md = "---\nname: review\ndescription: \"when reviewing\"\n---\n\ncite the file";
        s.take(word(
            "review",
            Some(skill(
                "review",
                &[("SKILL.md", md), ("refs/style.md", "short")],
            )),
            1,
            ShareScope::default(),
        ));
        // A header that asks for tools or commands, or no SKILL.md, is never given; each is said once.
        s.take(word(
            "tools",
            Some(skill(
                "tools",
                &[("SKILL.md", "---\nname: tools\nallowed-tools: Bash\n---\n")],
            )),
            1,
            ShareScope::default(),
        ));
        s.take(word(
            "bare",
            Some(skill("bare", &[("notes.md", "x")])),
            1,
            ShareScope::default(),
        ));
        s.take(word(
            "db",
            Some(item("db", "npx", "")),
            1,
            ShareScope::default(),
        ));
        let servers = s.servers_for("agent-1", "p", "claude-code", Path::new("/run/m/a.sock"));
        let dir = s.skills_dir("agent-1");
        std::fs::create_dir_all(dir.join("stale")).unwrap();
        let skills = s.skills_for("agent-1", "p", "claude-code", layout);
        assert_eq!(servers.len(), 1);
        assert_eq!(
            skills.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            ["review"]
        );
        let names = |agent: &str| {
            let mut n: Vec<String> = s.given(agent).into_iter().map(|r| r.name).collect();
            n.sort();
            n
        };
        assert_eq!(names("agent-1"), ["db", "review"]);
        let mut said: Vec<String> = s
            .take_events()
            .into_iter()
            .filter_map(|e| match e {
                Body::ShareItemRefused(r) => Some(format!("{}:{}", r.name, r.reason)),
                _ => None,
            })
            .collect();
        said.sort();
        assert_eq!(said, ["bare:skill_header", "tools:skill_header"]);
        s.skills_for("agent-1", "p", "claude-code", layout);
        assert!(s.take_events().is_empty(), "said once");

        assert!(!dir.join("stale").exists(), "what was there is replaced");
        assert_eq!(
            std::fs::read_to_string(dir.join(".claude-plugin/plugin.json")).unwrap(),
            r#"{"name":"shared"}"#
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("skills/review/SKILL.md")).unwrap(),
            md
        );
        assert!(dir.join("skills/review/refs/style.md").is_file());
        for (name, path) in [
            ("review", "../out.md"),
            ("review", "/etc/x"),
            ("../x", "SKILL.md"),
        ] {
            let mut bad = skills.clone();
            bad[0].0 = name.into();
            bad[0].1.files[0].path = path.into();
            assert!(write_skills(&dir, layout, &bad).is_err(), "{name} {path}");
        }
        assert!(!d.join("out.md").exists());
        s.agent_ended("agent-1");
        assert!(!dir.exists());
        assert!(s.given("agent-1").is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// ADR-0122 step 5: a skills folder that cannot be written gives no skill (the agent starts without them) and says so once.
    #[test]
    fn a_skills_folder_that_cannot_be_written_gives_none_and_says_so() {
        let d = temp("skills-fail");
        let s = open(&d, SharePolicy::default());
        let item = ShareItem {
            name: "review".into(),
            kind: ShareItemKind::Skill as i32,
            mcp: None,
            skill: Some(v1::SkillDef {
                description: "x".into(),
                files: vec![v1::SkillFile {
                    path: "SKILL.md".into(),
                    content: b"---\nname: review\n---\n".to_vec(),
                }],
            }),
        };
        s.take(word("review", Some(item), 1, ShareScope::default()));
        let blocker = s.skills_dir("agent-1");
        std::fs::create_dir_all(blocker.parent().unwrap()).unwrap();
        // A file where the agent's folder's parent must be a folder.
        let parent = blocker.parent().unwrap().to_path_buf();
        std::fs::remove_dir_all(&parent).unwrap();
        std::fs::write(&parent, b"in the way").unwrap();
        let got = s.skills_for(
            "agent-1",
            "p",
            "claude-code",
            agora_adapters::manifest::SkillsLayout::ClaudePlugin,
        );
        assert!(got.is_empty());
        assert!(s.given("agent-1").is_empty());
        let said = s.take_events();
        assert!(
            matches!(&said[..], [Body::ShareItemRefused(r)] if r.reason == "skill_not_written"),
            "{said:?}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// ADR-0122 step 5: only a `SKILL.md` whose header holds its name and description (comments, blank and continuation lines aside)
    /// is read as asking for nothing more.
    #[test]
    fn a_skill_header_names_only_the_skill_and_when_to_use_it() {
        let skill = |text: &str| v1::SkillDef {
            description: String::new(),
            files: vec![v1::SkillFile {
                path: "SKILL.md".into(),
                content: text.as_bytes().to_vec(),
            }],
        };
        for ok in [
            "---\nname: r\ndescription: x\n---\nbody",
            "---\r\nname: r\r\ndescription: >\r\n  folded\r\n# note\r\n\r\n---\r\n",
        ] {
            assert!(skill_header_ok(&skill(ok)), "{ok:?}");
        }
        for bad in [
            "no header",
            "\n---\nname: r\n---\n",
            "---\nname: r\nallowed-tools: Bash\n---\n",
            "---\nname: r\nhooks:\n  x: y\n---\n",
            "---\n{allowed-tools: Bash}\n---\n",
            "---\n? allowed-tools\n---\n",
            "---\nname: r\n",
            "---\nnamex: r\n---\n",
            "---\n allowed-tools: Bash\n name: x\n description: y\n---\n",
            "---\n# note\n  allowed-tools: Bash\nname: x\n---\n",
            "---\n---\n",
        ] {
            assert!(!skill_header_ok(&skill(bad)), "{bad:?}");
        }
        let mut lower = skill("---\nname: r\n---\n");
        lower.files[0].path = "skill.md".into();
        assert!(!skill_header_ok(&lower), "the file is SKILL.md exactly");
    }

    /// ADR-0122 Amendment A: a program that names a secret is given through the launcher only where this machine lets that secret
    /// go and holds it; the agent's file names the launcher and a single-use id, never the secret; the launch fills it, once.
    #[test]
    fn a_server_naming_a_secret_goes_through_the_launcher_and_only_where_the_machine_allows_it() {
        let d = temp("launcher");
        let with_db = |allowed: &[&str]| {
            let s = open(
                &d,
                SharePolicy {
                    secrets: allowed.iter().map(|x| x.to_string()).collect(),
                    ..SharePolicy::default()
                },
            );
            let mut db = item("db", "npx", "");
            db.mcp.as_mut().unwrap().args = vec!["-y".into(), "db-mcp".into()];
            db.mcp.as_mut().unwrap().env = vec![v1::ShareValue {
                name: "DB_URL".into(),
                value: "{secret:DB}?ssl=1".into(),
            }];
            s.take(word("db", Some(db), 1, ShareScope::default()));
            s
        };
        let sock = Path::new("/run/m/agent-1.sock");
        // Not listed in `share.secrets`: refused, whatever the owner signed.
        let s = with_db(&[]);
        s.set_secrets(Box::new(|n| {
            (n == "DB").then(|| "postgres://x".to_string())
        }));
        assert!(s.servers_for("agent-1", "p", "k", sock).is_empty());
        assert!(
            matches!(&s.take_events()[..], [Body::ShareItemRefused(r)] if r.reason == "secret_not_allowed")
        );
        // Listed but not held: refused, naming the secret.
        let s = with_db(&["DB"]);
        s.set_secrets(Box::new(|_| None));
        assert!(s.servers_for("agent-1", "p", "k", sock).is_empty());
        assert!(
            matches!(&s.take_events()[..], [Body::ShareItemRefused(r)] if r.code == "share.secret_missing" && r.reason == "DB")
        );
        // Listed and held: the launcher, a single-use id, no environment in the file.
        let s = with_db(&["DB"]);
        s.set_secrets(Box::new(|n| {
            (n == "DB").then(|| "postgres://x".to_string())
        }));
        let got = s.servers_for("agent-1", "p", "k", sock);
        assert_eq!(got.len(), 1);
        let server = &got[0];
        assert!(server.env.is_empty() && server.url.is_empty());
        assert_eq!(
            &server.args[..4],
            ["mcp-launch", "--socket", "/run/m/agent-1.sock", "db"]
        );
        let id = server.args[4].clone();
        assert_eq!(id.len(), 64);
        assert!(
            !format!("{server:?}").contains("postgres"),
            "the secret is in no part of the file"
        );
        assert_eq!(
            s.launch("agent-2", "db", &id),
            Err("unknown_launch"),
            "another agent's ask spends nothing it may use"
        );
        let got = s.servers_for("agent-1", "p", "k", sock);
        let id = got[0].args[4].clone();
        let plan = s.launch("agent-1", "db", &id).unwrap();
        assert_eq!(
            (plan.command.as_str(), plan.args.as_slice()),
            ("npx", &["-y".to_string(), "db-mcp".to_string()][..])
        );
        assert_eq!(
            plan.env,
            [("DB_URL".to_string(), "postgres://x?ssl=1".to_string())]
        );
        assert_eq!(
            s.launch("agent-1", "db", &id),
            Err("instance_live"),
            "one instance at a time (ADR-0133)"
        );
        s.instance_ended("agent-1", "db", &id, true);
        // Only the newest launch's id for an item is live.
        let first = s.servers_for("agent-1", "p", "k", sock)[0].args[4].clone();
        let second = s.servers_for("agent-1", "p", "k", sock)[0].args[4].clone();
        assert_eq!(
            s.launch("agent-1", "db", &first),
            Err("unknown_launch"),
            "replaced by the next launch"
        );
        assert!(s.launch("agent-1", "db", &second).is_ok());
        // A new version of the item since the agent's launch: its id gets nothing.
        let id = s.servers_for("agent-1", "p", "k", sock)[0].args[4].clone();
        let mut db2 = item("db", "npx", "");
        db2.mcp.as_mut().unwrap().env = vec![v1::ShareValue {
            name: "DB_URL".into(),
            value: "{secret:DB}".into(),
        }];
        s.take(word("db", Some(db2), 2, ShareScope::default()));
        assert_eq!(
            s.launch("agent-1", "db", &id),
            Err("unknown_item"),
            "another version"
        );
        // A scope narrowed since the agent's launch so it no longer covers it: nothing.
        let id = s.servers_for("agent-1", "p", "k", sock)[0].args[4].clone();
        let mut db3 = item("db", "npx", "");
        db3.mcp.as_mut().unwrap().env = vec![v1::ShareValue {
            name: "DB_URL".into(),
            value: "{secret:DB}".into(),
        }];
        s.take(word(
            "db",
            Some(db3),
            3,
            ShareScope {
                project_id: "other".into(),
                kinds: Vec::new(),
            },
        ));
        assert_eq!(
            s.launch("agent-1", "db", &id),
            Err("unknown_item"),
            "no longer in its scope"
        );
        // An agent that ended has no launches left (a fresh folder: the one above remembers the newer words).
        let d2 = temp("launcher-end");
        let s = open(
            &d2,
            SharePolicy {
                secrets: ["DB".to_string()].into_iter().collect(),
                ..SharePolicy::default()
            },
        );
        let mut db = item("db", "npx", "");
        db.mcp.as_mut().unwrap().env = vec![v1::ShareValue {
            name: "DB_URL".into(),
            value: "{secret:DB}".into(),
        }];
        s.take(word("db", Some(db), 1, ShareScope::default()));
        s.set_secrets(Box::new(|n| {
            (n == "DB").then(|| "postgres://x".to_string())
        }));
        let got = s.servers_for("agent-1", "p", "k", sock);
        let id = got[0].args[4].clone();
        s.agent_ended("agent-1");
        assert_eq!(s.launch("agent-1", "db", &id), Err("unknown_launch"));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&d2);
    }

    /// ADR-0133: a binding lasts the agent's run: one instance at a time, a new one after a clean end, a pause after a bad end, at most
    /// six starts a minute; a launcher gone while its server (a process this node can tell) still runs keeps the binding taken; a
    /// removed item or an ended agent tells the launcher to close.
    #[test]
    fn a_shared_server_restarts_within_its_agents_run_within_bounds() {
        let d = temp("restart");
        let s = open(
            &d,
            SharePolicy {
                secrets: ["DB".to_string()].into_iter().collect(),
                ..SharePolicy::default()
            },
        );
        let mut db = item("db", "npx", "");
        db.mcp.as_mut().unwrap().env = vec![v1::ShareValue {
            name: "DB_URL".into(),
            value: "{secret:DB}".into(),
        }];
        s.take(word("db", Some(db), 1, ShareScope::default()));
        s.set_secrets(Box::new(|n| {
            (n == "DB").then(|| "postgres://x".to_string())
        }));
        let sock = Path::new("/run/m/agent-1.sock");
        let id = s.servers_for("agent-1", "p", "k", sock)[0].args[4].clone();
        // Clean ends: a new instance each time, up to the budget.
        for _ in 0..STARTS_PER_MINUTE {
            assert!(s.launch("agent-1", "db", &id).is_ok());
            assert_eq!(s.launch("agent-1", "db", &id), Err("instance_live"));
            s.instance_ended("agent-1", "db", &id, true);
        }
        assert_eq!(s.launch("agent-1", "db", &id), Err("start_budget"));
        // A bad end, or a launcher gone with no server known, makes the next start wait.
        let id = s.servers_for("agent-1", "p", "k", sock)[0].args[4].clone();
        assert!(s.launch("agent-1", "db", &id).is_ok());
        s.instance_ended("agent-1", "db", &id, false);
        assert_eq!(s.launch("agent-1", "db", &id), Err("too_soon"));
        let id = s.servers_for("agent-1", "p", "k", sock)[0].args[4].clone();
        assert!(s.launch("agent-1", "db", &id).is_ok());
        s.instance_dropped("agent-1", "db", &id);
        assert_eq!(s.launch("agent-1", "db", &id), Err("too_soon"));
        // A launcher gone while its server still runs (this test's own process stands for it): still taken.
        let id = s.servers_for("agent-1", "p", "k", sock)[0].args[4].clone();
        assert!(s.launch("agent-1", "db", &id).is_ok());
        s.instance_started("agent-1", "db", &id, std::process::id());
        s.instance_dropped("agent-1", "db", &id);
        if agora_platform::process_identity(std::process::id()).is_ok() {
            assert_eq!(s.launch("agent-1", "db", &id), Err("instance_live"));
        }
        // Another agent's notices change nothing; removal and the agent's end tell the launcher to close.
        s.instance_ended("agent-2", "db", &id, true);
        assert!(!s.instance_revoked("agent-1", "db", &id));
        assert!(s.instance_revoked("agent-2", "db", &id));
        s.take(word("db", None, 2, ShareScope::default()));
        assert!(s.instance_revoked("agent-1", "db", &id), "removed");
        s.agent_ended("agent-1");
        assert!(s.instance_revoked("agent-1", "db", &id), "the agent ended");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// An item may not set what a loader or an interpreter reads first: an allowed program would then run anything.
    #[test]
    fn an_item_that_sets_a_loader_or_interpreter_variable_is_refused() {
        let on = SharePolicy::default();
        let trusted = |_: &str, _: &[u8], _: &[u8]| true;
        for name in [
            "npm_config_script_shell",
            "GCONV_PATH",
            "LD_PRELOAD",
            "ld_library_path",
            "DYLD_INSERT_LIBRARIES",
            "NODE_OPTIONS",
            "PYTHONPATH",
            "PATH",
            "Path",
            "BASH_ENV",
        ] {
            let mut it = item("db", "npx", "");
            it.mcp.as_mut().unwrap().env = vec![v1::ShareValue {
                name: name.into(),
                value: "x".into(),
            }];
            let w = word("db", Some(it), 1, ShareScope::default());
            assert_eq!(
                judge(&w, None, &on, trusted).unwrap_err().reason,
                "env_not_allowed",
                "{name}"
            );
        }
        let mut it = item("db", "npx", "");
        it.mcp.as_mut().unwrap().env = vec![v1::ShareValue {
            name: "DB_MODE".into(),
            value: "ro".into(),
        }];
        assert_eq!(
            judge(
                &word("db", Some(it), 1, ShareScope::default()),
                None,
                &on,
                trusted
            ),
            Ok(())
        );
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
