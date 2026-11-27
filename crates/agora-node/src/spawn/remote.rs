//! ADR-0119 step 2: the target node's side of a helper another machine asks for.
//!
//! A forwarded `HelperStart` (only ever from the hub link, never from a local client) is checked against this node's own policy,
//! manifests and limits, then either started at once under a live standing grant or put to the person as an `agent.start.remote`
//! card bound to the asking machine and agent. The decision is verified over `agora-approval-start-v4` (`crate::approvals`); an
//! approved one launches the helper as any helper is launched here, with its parent on the other machine, and may leave a standing
//! grant bounded by what was signed. Every outcome is queued as a `HelperStartResult` for the hub to carry back.

use super::*;
use crate::approvals::{remote_grant_end, RemoteBinding};
use crate::daemon::RemoteParent;

/// An asker this deep may not ask (the person's agent is 0; its helper 1; down to 3).
pub const MAX_REMOTE_DEPTH: u32 = 3;
/// Cards waiting at once per asking machine, and helpers running per asking machine and per asking run, without a grant.
pub const CARDS_PER_ORIGIN: usize = 2;
pub const RUNNING_PER_ORIGIN: usize = 6;
pub const RUNNING_PER_RUN: usize = 3;
/// Starts a standing grant covers in any hour.
pub const GRANT_STARTS_PER_HOUR: usize = 30;

/// Who asked, as the hub carried it: the machine (`origin`, the hub's word), its agent and run, and the chain's root (the hub's word).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Asker {
    origin: String,
    agent: String,
    run: Vec<u8>,
    kind: String,
    depth: u32,
    root_node: String,
    root_agent: String,
    root_run: Vec<u8>,
}

impl Asker {
    fn from(h: &v1::HelperStart) -> Self {
        Self {
            origin: h.origin.clone(),
            agent: h.from_agent.clone(),
            run: h.from_run_id.clone(),
            kind: h.from_agent_kind.clone(),
            depth: h.depth,
            root_node: h.root_node.clone(),
            root_agent: h.root_agent.clone(),
            root_run: h.root_run_id.clone(),
        }
    }

    /// The name the asker goes by on this node's records (`<origin>/<agent>`).
    fn id(&self) -> AgentId {
        AgentId::new(format!("{}/{}", self.origin, self.agent))
    }

    fn same_run(&self, other: &Asker) -> bool {
        self.origin == other.origin && self.agent == other.agent && self.run == other.run
    }

    fn same_root(&self, other: &Asker) -> bool {
        self.root_node == other.root_node
            && self.root_agent == other.root_agent
            && self.root_run == other.root_run
    }
}

/// A card waiting for the person.
pub(super) struct Waiting {
    start_id: Vec<u8>,
    asker: Asker,
    req: Request,
}

/// A helper started here for another machine.
pub(super) struct Started {
    asker: Asker,
    grant: String,
}

/// A standing grant the person gave on a card.
pub(super) struct Grant {
    id: String,
    signer: String,
    asker: Asker,
    chain: bool,
    kind: String,
    any_kind: bool,
    folder: String,
    below_folder: bool,
    max_running: u32,
    end_ms: u64,
    starts: VecDeque<Instant>,
}

#[derive(Default)]
pub(super) struct RemoteState {
    /// Whether this node takes starts from other machines (`mesh.remote_helpers`, and `mesh.remote`).
    on: bool,
    waiting: HashMap<String, Waiting>,
    started: HashMap<AgentId, Started>,
    grants: Vec<Grant>,
    /// Results for the hub to carry back to the asking machines.
    outbox: Vec<v1::HelperStartResult>,
}

fn policy_off(node: &str) -> Error {
    Error::new(ErrorCode::NodePolicyDenied)
        .with("node_id", node)
        .with("rule", "mesh.remote_helpers")
}

impl Spawner {
    /// ADR-0119: whether this node takes starts from other machines (`mesh.remote_helpers` and `mesh.remote` in its policy).
    pub fn set_remote_helpers(&self, on: bool) {
        self.st().remote.on = on;
    }

    /// The helpers started here for other machines that still run, by asker.
    fn remote_running(&self, st: &State, views: &[AgentView]) -> Vec<(AgentId, Asker, String)> {
        views
            .iter()
            .filter(|v| v.running)
            .filter_map(|v| {
                st.remote
                    .started
                    .get(&v.id)
                    .map(|s| (v.id.clone(), s.asker.clone(), s.grant.clone()))
            })
            .collect()
    }

    /// Everything decided before the person is asked, as for a local start, from the target's own manifests and policy.
    fn remote_check(&self, h: &v1::HelperStart, asker: &Asker) -> Result<Request, Error> {
        if asker.origin.is_empty() {
            return Err(invalid("origin"));
        }
        if asker.depth >= MAX_REMOTE_DEPTH {
            return Err(helper_limit("depth", MAX_REMOTE_DEPTH as usize));
        }
        check_prompt(&h.prompt)?;
        let (kinds, confined, trusted, markers) = {
            let sup = self.sup();
            let p = sup.sandbox_policy();
            (
                sup.kinds(),
                p.level != crate::sandbox::Level::Off,
                p.trusted_kinds.clone(),
                sup.typed_markers(&h.kind),
            )
        };
        if carries_marker(&h.prompt, &markers) {
            return Err(invalid("prompt"));
        }
        if !kinds.iter().any(|k| k.kind == h.kind) {
            return Err(
                Error::new(ErrorCode::AgentKindUnknown).with("kind", cut(&h.kind, MAX_FIELD))
            );
        }
        let pairs: Vec<(String, String)> = h
            .settings
            .iter()
            .map(|s| (s.id.clone(), s.value.clone()))
            .collect();
        let settings = self
            .sup()
            .checked_settings(&h.kind, &pairs)
            .map_err(|e| e.to_protocol())?;
        let options = self
            .sup()
            .checked_options(&h.kind, &h.launch_options)
            .map_err(|e| e.to_protocol())?;
        // The ceiling on confinement: the asker's kind cannot be compared across machines, so a kind whose manifest the owner trusts to
        // widen confinement is never started for another machine while agents are confined here.
        if confined && trusted.contains(&h.kind) {
            return Err(Error::new(ErrorCode::NodePolicyDenied)
                .with("node_id", self.node_id.as_str())
                .with("rule", "sandbox.trust_manifest"));
        }
        let folder_refused =
            |f: &str| Error::new(ErrorCode::MeshFolderNotAllowed).with("folder", cut(f, MAX_FIELD));
        // A full path on this machine: the asker's folder means nothing here.
        if h.folder.is_empty()
            || !Path::new(&h.folder).is_absolute()
            || h.folder.chars().any(|c| c.is_control() || invisible(c))
        {
            return Err(folder_refused(&h.folder));
        }
        let folder = agora_platform::canonicalize_plain(Path::new(&h.folder))
            .map_err(|_| folder_refused(&h.folder))?;
        let text = folder
            .to_str()
            .ok_or_else(|| folder_refused(&folder.to_string_lossy()))?;
        let (info, _) = crate::directory::inspect(text, &self.node_id, false)?;
        if info.path.chars().any(|c| c.is_control() || invisible(c)) {
            return Err(folder_refused(&info.path));
        }
        if h.worktree && info.git_root.is_empty() {
            return Err(invalid("directory.git_root"));
        }
        let grantable = self.sup().grantable(&h.kind);
        Ok(Request {
            caller: asker.id(),
            kind: h.kind.clone(),
            folder: info.path,
            inside: false,
            worktree: h.worktree,
            prompt: h.prompt.clone(),
            settings,
            options,
            depth: u8::try_from(asker.depth + 1).unwrap_or(u8::MAX),
            grantable,
            ticket: 0,
        })
    }

    /// The live grant that covers this start, if any (its id). Ended grants are dropped on the way.
    fn covering_grant(
        st: &mut State,
        running: &[(AgentId, Asker, String)],
        asker: &Asker,
        req: &Request,
        now_ms: u64,
    ) -> Option<String> {
        st.remote.grants.retain(|g| now_ms < g.end_ms);
        // Settings and launch options are always asked, as locally (a grant approved a start as shown).
        if !req.settings.is_empty() || !req.options.is_empty() {
            return None;
        }
        let now = Instant::now();
        for g in &mut st.remote.grants {
            let who = if g.chain {
                !g.asker.root_node.is_empty() && g.asker.same_root(asker)
            } else {
                g.asker.same_run(asker)
            };
            let what = if g.any_kind {
                req.grantable
            } else {
                g.kind == req.kind
            };
            let place = if g.below_folder {
                within(Path::new(&req.folder), Path::new(&g.folder))
            } else {
                req.folder == g.folder
            };
            if !(who && what && place) {
                continue;
            }
            while g
                .starts
                .front()
                .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(3600))
            {
                g.starts.pop_front();
            }
            let in_use = running.iter().filter(|(_, _, id)| *id == g.id).count();
            if in_use >= g.max_running as usize || g.starts.len() >= GRANT_STARTS_PER_HOUR {
                continue;
            }
            g.starts.push_back(now);
            return Some(g.id.clone());
        }
        None
    }

    /// ADR-0119 decision 3: a forwarded `HelperStart`. Answered with the card's id, or with the grant it started under; the outcome
    /// follows as a `HelperStartResult` ([`Self::take_remote_results`]).
    pub fn remote_start(
        &self,
        h: &v1::HelperStart,
        now_ms: u64,
    ) -> Result<v1::HelperStartAccepted, Error> {
        self.stopping()?;
        if !self.st().remote.on {
            return Err(policy_off(&self.node_id));
        }
        if h.start_id.len() != 16 {
            return Err(invalid("start_id"));
        }
        let asker = Asker::from(h);
        let req = self.remote_check(h, &asker)?;
        let views = self.sup().list();
        let granted = {
            let mut st = self.st();
            let running = self.remote_running(&st, &views);
            Self::covering_grant(&mut st, &running, &asker, &req, now_ms)
        };
        if let Some(grant_id) = granted {
            let parent = RemoteParent {
                node: asker.origin.clone(),
                agent: asker.agent.clone(),
                grant_id: grant_id.clone(),
            };
            let may_start = self.grant_allows_chain(&grant_id);
            let outcome = self.launch_as(&req, may_start, Some(&parent));
            self.finish_remote(&h.start_id, &asker, &grant_id, outcome);
            return Ok(v1::HelperStartAccepted {
                approval_id: String::new(),
                ttl_ms: 0,
                granted: true,
                grant_id,
            });
        }
        {
            let st = self.st();
            let running = self.remote_running(&st, &views);
            let cards = st
                .remote
                .waiting
                .values()
                .filter(|w| w.asker.origin == asker.origin)
                .count();
            if cards >= CARDS_PER_ORIGIN {
                return Err(helper_limit("pending", CARDS_PER_ORIGIN));
            }
            if running
                .iter()
                .filter(|(_, a, _)| a.origin == asker.origin)
                .count()
                >= RUNNING_PER_ORIGIN
            {
                return Err(helper_limit("helpers", RUNNING_PER_ORIGIN));
            }
            if running
                .iter()
                .filter(|(_, a, _)| a.same_run(&asker))
                .count()
                >= RUNNING_PER_RUN
            {
                return Err(helper_limit("helpers", RUNNING_PER_RUN));
            }
        }
        let detail = self.remote_detail(&req, &asker);
        let Some(p) = self.approvals.request_remote_start(
            &detail,
            RemoteBinding {
                origin_node: asker.origin.clone(),
                origin_agent: asker.agent.clone(),
            },
            now_ms,
        ) else {
            return Err(Error::new(ErrorCode::Internal));
        };
        let ttl = p.expires_ms.saturating_sub(now_ms);
        // Filed before it is announced, so a decision that comes at once finds it.
        self.st().remote.waiting.insert(
            p.id.clone(),
            Waiting {
                start_id: h.start_id.clone(),
                asker,
                req,
            },
        );
        if let Some(ev) = &self.events {
            ev.approval_requested(&p);
        }
        Ok(v1::HelperStartAccepted {
            approval_id: p.id,
            ttl_ms: ttl,
            granted: false,
            grant_id: String::new(),
        })
    }

    fn grant_allows_chain(&self, grant_id: &str) -> bool {
        self.st()
            .remote
            .grants
            .iter()
            .any(|g| g.id == grant_id && g.chain)
    }

    /// ADR-0119 decision 4: the card's detail, what the person reads and signs (by hash).
    fn remote_detail(&self, r: &Request, asker: &Asker) -> String {
        let mut detail = json!({
            "node": self.node_id,
            "origin_node": asker.origin,
            "origin_agent": asker.agent,
            "origin_agent_kind": asker.kind,
            "kind": r.kind,
            "folder": self.chat.redact_text(&r.folder),
            "worktree": r.worktree,
            "depth": asker.depth,
            "root_node": asker.root_node,
            "root_agent": asker.root_agent,
            "grant_allowed": r.settings.is_empty() && r.options.is_empty() && r.grantable,
            "prompt": self.chat.redact_text(&r.prompt),
        });
        if !r.settings.is_empty() {
            detail["settings"] = r
                .settings
                .iter()
                .map(|(id, label, value)| json!({"id": id, "label": label, "value": value}))
                .collect();
        }
        if !r.options.is_empty() {
            detail["launch_options"] = r
                .options
                .iter()
                .map(|(id, label, skips)| json!({"id": id, "label": label, "skips_permissions": skips}))
                .collect();
        }
        detail.to_string()
    }

    /// The person's verified decision on an `agent.start.remote` card (called after the v4 signature was verified).
    pub fn remote_decided(
        &self,
        request: &Pending,
        r: &v1::ApprovalResolve,
        now_ms: u64,
    ) -> Result<(), Error> {
        let Some(w) = self.st().remote.waiting.remove(&request.id) else {
            return Err(
                Error::new(ErrorCode::MeshStartDenied).with("approval_id", request.id.as_str())
            );
        };
        let grant = r.remote_grant.unwrap_or_default();
        let end = remote_grant_end(&grant, now_ms).ok().flatten();
        let mut grant_id = String::new();
        let outcome = if r.approved {
            let parent = RemoteParent {
                node: w.asker.origin.clone(),
                agent: w.asker.agent.clone(),
                grant_id: String::new(),
            };
            let chain = end.is_some() && grant.chain;
            let outcome = self.launch_as(&w.req, chain, Some(&parent));
            if let (Ok(_), Some(end_ms)) = (&outcome, end) {
                grant_id = request.id.clone();
                self.st().remote.grants.push(Grant {
                    id: request.id.clone(),
                    signer: r.signer_device_id.clone(),
                    asker: w.asker.clone(),
                    chain: grant.chain,
                    kind: w.req.kind.clone(),
                    any_kind: grant.any_kind,
                    folder: w.req.folder.clone(),
                    below_folder: grant.below_folder,
                    max_running: grant.max_running,
                    end_ms,
                    starts: VecDeque::from([Instant::now()]),
                });
            }
            outcome
        } else {
            Err(Error::new(ErrorCode::MeshStartDenied).with("approval_id", request.id.as_str()))
        };
        if let Some(ev) = &self.events {
            ev.approval_resolved(&request.id, r.approved, &r.signer_device_id);
        }
        // The helper itself carries no grant: the grant it may have created covers later starts.
        self.finish_remote(&w.start_id, &w.asker, &grant_id, outcome);
        Ok(())
    }

    /// Records a remote start's outcome and queues its result for the asking machine.
    fn finish_remote(&self, start_id: &[u8], asker: &Asker, grant_id: &str, outcome: StartOutcome) {
        let mut st = self.st();
        let result = match outcome {
            Ok(id) => {
                let agent = AgentId::new(id.clone());
                st.remote.started.insert(
                    agent.clone(),
                    Started {
                        asker: asker.clone(),
                        grant: grant_id.to_string(),
                    },
                );
                v1::HelperStartResult {
                    start_id: start_id.to_vec(),
                    to_node: asker.origin.clone(),
                    terminal_id: crate::control::terminal_id_of(&agent),
                    agent_id: id,
                    error: None,
                    from_node: String::new(),
                }
            }
            Err(e) => v1::HelperStartResult {
                start_id: start_id.to_vec(),
                to_node: asker.origin.clone(),
                agent_id: String::new(),
                terminal_id: String::new(),
                error: Some(e.to_info()),
                from_node: String::new(),
            },
        };
        st.remote.outbox.push(result);
    }

    /// A card that ran out of time: its asker hears `approval.expired`. `false` if it was not a remote card.
    pub fn remote_expired(&self, approval_id: &str) -> bool {
        let Some(w) = self.st().remote.waiting.remove(approval_id) else {
            return false;
        };
        if let Some(ev) = &self.events {
            ev.approval_resolved(approval_id, false, "");
        }
        self.finish_remote(
            &w.start_id,
            &w.asker,
            "",
            Err(Error::new(ErrorCode::ApprovalExpired).with("approval_id", approval_id)),
        );
        true
    }

    /// The results waiting to be carried back (each is taken once; the caller sends them to the hub).
    pub fn take_remote_results(&self) -> Vec<v1::HelperStartResult> {
        std::mem::take(&mut self.st().remote.outbox)
    }

    /// The live standing grants, for `GrantList`.
    pub fn remote_grants(&self, now_ms: u64) -> Vec<v1::RemoteGrantInfo> {
        let views = self.sup().list();
        let mut st = self.st();
        st.remote.grants.retain(|g| now_ms < g.end_ms);
        let running = self.remote_running(&st, &views);
        let now = Instant::now();
        st.remote
            .grants
            .iter()
            .map(|g| v1::RemoteGrantInfo {
                grant_id: g.id.clone(),
                signer_device_id: g.signer.clone(),
                origin_node: g.asker.origin.clone(),
                origin_agent: g.asker.agent.clone(),
                root_node: g.asker.root_node.clone(),
                root_agent: g.asker.root_agent.clone(),
                kind: g.kind.clone(),
                folder: g.folder.clone(),
                any_kind: g.any_kind,
                below_folder: g.below_folder,
                chain: g.chain,
                max_running: g.max_running,
                running: u32::try_from(running.iter().filter(|(_, _, id)| *id == g.id).count())
                    .unwrap_or(u32::MAX),
                starts_this_hour: u32::try_from(
                    g.starts
                        .iter()
                        .filter(|t| now.duration_since(**t) < Duration::from_secs(3600))
                        .count(),
                )
                .unwrap_or(u32::MAX),
                until_ms: g.end_ms,
            })
            .collect()
    }

    /// Ends a standing grant by id; whether there was one.
    pub fn revoke_remote_grant(&self, id: &str) -> bool {
        let mut st = self.st();
        let before = st.remote.grants.len();
        st.remote.grants.retain(|g| g.id != id);
        before != st.remote.grants.len()
    }
}
