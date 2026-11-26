//! ADR-0039: the node renews the shared-board leases of its agents while they run, so a lease on a shared project lasts until its
//! agent releases it or ends, and its TTL only bounds how long it outlives a machine that went away (the hub ends it when the TTL
//! passes with no renewal). Leases on the node's own broker (a project only this machine has) are not renewed: ADR-0039 leaves
//! local-only projects untouched. They are remembered, though, so that when the project becomes shared (`BoardJoin`, ADR-0039
//! amendment B) the leases the node hands over are renewed on the board from then on.
//!
//! This is the bookkeeping only: plain data and an injected clock, no I/O, no locks. The clock is the node's monotonic one in
//! milliseconds (never the wall clock, which can jump); only the file the rows are kept in across a node restart is anchored to the
//! wall clock ([`encode`], [`decode`]). The mesh server records what the hub's board granted an agent, takes the renewals that are
//! [`Renewals::due`] from its ticker, sends each as the agent's own `lease_acquire` on the board (the only lease call the board
//! has, so no new protocol message), and gives the answer back ([`Renewals::answered`]).
//!
//! A renewal is the board's `lease_acquire`, which gives the paths a new lease id. The agent keeps the id it was given: the node
//! remembers, per grant, the id the agent knows and the one the board has now, and puts the current one into the agent's
//! `lease_release` ([`Renewals::release_target`]).
//!
//! Schedule ([`Policy`]): a grant is renewed when half its TTL has passed since the call that last granted it was sent, and only a
//! TTL of at least 30 s is renewed (so at most once every 15 s; a shorter one ends as asked). An agent has at most 256 paths and 64
//! grants renewed; a grant beyond that ends at its TTL. A renewal that fails for a reason that may pass (the hub cannot be reached,
//! it timed out, the board is taking over a project's work) is tried again every second until the grant's end as the node reckons
//! it; after that the lease is gone (a node that was offline does not take it back on reconnect). One that the board refuses
//! (another agent holds the path, the agent no longer runs in the hub's view, or the board no longer has the project) ends the
//! grant at once.

use std::collections::{HashMap, HashSet};

use agora_core::{AgentId, LeaseTable};
use serde_json::{json, Value};

/// How the node renews (the defaults are the node's; a test may shorten them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// A lease with a shorter TTL is not renewed: it ends as asked.
    pub min_ttl_ms: u64,
    /// No grant is renewed more often than this.
    pub min_gap_ms: u64,
    /// At most this many paths of one agent are renewed, over all its grants.
    pub max_paths_per_agent: usize,
    /// At most this many grants of one agent are renewed.
    pub max_grants_per_agent: usize,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            min_ttl_ms: 30_000,
            min_gap_ms: 15_000,
            max_paths_per_agent: 256,
            max_grants_per_agent: 64,
        }
    }
}

/// How long after a renewal that may succeed later failed it is tried again.
pub const RETRY_MS: u64 = 1_000;
/// A renewal with no answer after this long is taken as failed (longer than the uplink's 40 s deadline for a post, which answers
/// first, so this only covers a link that never answers).
pub const IN_FLIGHT_MS: u64 = 45_000;
/// A lease id longer than this is not recorded (ids are short; the board's answer is data from another machine).
const MAX_ID_BYTES: usize = 128;
/// The kept file is read only up to this size, and at most this many rows are taken from it.
pub const MAX_FILE_BYTES: u64 = 4 << 20;
const MAX_ROWS: usize = 16_384;
/// The version of the kept file.
const VERSION: u64 = 1;

/// One renewal to send: `lease_acquire` with `arguments` on `project`'s board, as `agent`.
#[derive(Debug, Clone, PartialEq)]
pub struct Renewal {
    pub key: u64,
    pub agent: AgentId,
    pub project: String,
    pub arguments: Value,
    /// When it was handed out to be sent: the grant's new end is reckoned from here.
    pub sent_at_ms: u64,
}

/// What the board's answer to a renewal means for the grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Granted again under this id, for the grant's TTL.
    Renewed { lease_id: String },
    /// The board refused it for good: the lease is gone, stop renewing.
    Lost,
    /// Not decided (the hub could not be asked, timed out, or the board is busy): try again until the grant's end.
    Retry,
}

/// A board grant as it is kept across a node restart (the clock is the monotonic one of the run that holds it).
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub agent: AgentId,
    pub project: String,
    pub shown: String,
    pub current: String,
    pub paths: Vec<String>,
    pub ttl_s: Value,
    pub granted_at_ms: u64,
}

#[derive(Debug, Clone)]
struct Grant {
    key: u64,
    agent: AgentId,
    project: String,
    /// The lease id the agent was given and still uses.
    shown: String,
    /// The board's id for these paths now (changes on every renewal).
    current: String,
    /// As the lease table keeps them (normalized); the paths the agent took again under a newer grant are no longer here.
    paths: Vec<String>,
    /// The agent's own `ttl_s`, sent again unchanged.
    ttl_s: Value,
    ttl_ms: u64,
    /// When the call that last granted it was sent.
    granted_at_ms: u64,
    /// On the hub's board (renewed); `false`: on the node's own broker, remembered until the project becomes shared.
    on_board: bool,
    /// Moved to the board by `BoardJoin`: the board's id is not known yet, so it is renewed at once.
    due_now: bool,
    /// The agent asked for its release and the answer did not say it happened: no longer renewed, but its id is still translated
    /// so the agent's next try reaches the board's lease.
    releasing: bool,
    /// A renewal of it is on its way, sent at this time.
    in_flight_since: Option<u64>,
    /// After a failed renewal: not tried again before this.
    not_before_ms: u64,
}

impl Grant {
    /// The end as the node reckons it: never later than the hub's, which counts from when it received the call.
    fn ends_at(&self) -> u64 {
        self.granted_at_ms.saturating_add(self.ttl_ms)
    }

    /// When to renew it next; `None` when it is not renewed (not on the board, being released, or a TTL too short).
    fn renew_at(&self, policy: &Policy) -> Option<u64> {
        if !self.on_board || self.releasing || self.ttl_ms < policy.min_ttl_ms {
            return None;
        }
        if self.due_now {
            return Some(self.not_before_ms);
        }
        let gap = (self.ttl_ms / 2).max(policy.min_gap_ms);
        Some(
            self.granted_at_ms
                .saturating_add(gap)
                .max(self.not_before_ms),
        )
    }

    fn row(&self) -> Row {
        Row {
            agent: self.agent.clone(),
            project: self.project.clone(),
            shown: self.shown.clone(),
            current: self.current.clone(),
            paths: self.paths.clone(),
            ttl_s: self.ttl_s.clone(),
            granted_at_ms: self.granted_at_ms,
        }
    }
}

/// The seconds of a `ttl_s` argument in milliseconds, read as the broker reads it (finite, not negative); `None` otherwise.
pub fn ttl_ms(ttl_s: &Value) -> Option<u64> {
    let n = ttl_s.as_f64()?;
    // `as` saturates: a huge TTL is the longest one, never a wrapped short one.
    (n.is_finite() && n >= 0.0).then_some((n * 1000.0) as u64)
}

/// A path as the lease table keeps it (`./src//a.rs` is `src/a.rs`), with the table's own rule; `None` when the table would refuse
/// it.
fn canonical(path: &str) -> Option<String> {
    LeaseTable::new()
        .acquire(path, &AgentId::new("-"), 0, 1)
        .ok()
        .map(|l| l.path)
}

fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_BYTES
}

/// What the board answered a renewal (`Err` when the hub could not be asked, else whether it says the project is shared and the
/// tool's MCP result as JSON), as an [`Outcome`].
pub fn classify(reply: Result<(bool, &str), &str>) -> Outcome {
    let Ok((shared, result_json)) = reply else {
        return Outcome::Retry;
    };
    // The board no longer has the project: it holds no live lease, so this one ended.
    if !shared {
        return Outcome::Lost;
    }
    let Some((body, is_error)) = tool_body(result_json) else {
        return Outcome::Lost;
    };
    if is_error {
        return match error_code(&body) {
            Some(code) if retryable(code) => Outcome::Retry,
            _ => Outcome::Lost,
        };
    }
    match body.get("lease_id").and_then(Value::as_str) {
        Some(id) if id_ok(id) => Outcome::Renewed {
            lease_id: id.to_string(),
        },
        _ => Outcome::Lost,
    }
}

/// Refusals that change nothing and may pass: the hub's board is busy or cannot store, or the call could not be sent.
fn retryable(code: &str) -> bool {
    matches!(
        code,
        "board.unavailable"
            | "node.offline"
            | "node.timeout"
            | "internal"
            | "mesh.unavailable"
            | "mesh.rate_limited"
    )
}

/// An MCP tool result (as JSON text) as its first text item, parsed, and whether it is an error.
fn tool_body(result_json: &str) -> Option<(Value, bool)> {
    let result: Value = serde_json::from_str(result_json).ok()?;
    let text = result.get("content")?.get(0)?.get("text")?.as_str()?;
    let body: Value = serde_json::from_str(text).ok()?;
    Some((
        body,
        result.get("isError").and_then(Value::as_bool) == Some(true),
    ))
}

fn error_code(body: &Value) -> Option<&str> {
    body.get("error")?.get("code")?.as_str()
}

/// Whether the board's answer to an agent's own `lease_release` (`None`: no answer) says the lease is gone, so its renewal and id
/// are forgotten. A release the hub could not decide keeps the id, so the agent's next try still reaches the board's lease.
pub fn release_settled(reply: Option<(bool, &str)>) -> bool {
    let Some((true, result_json)) = reply else {
        return false;
    };
    match tool_body(result_json) {
        Some((_, false)) => true,
        Some((body, true)) => !error_code(&body).is_some_and(retryable),
        None => false,
    }
}

/// The board grants the node renews, the node's own broker grants it remembers for a `BoardJoin`, the rows kept from the last run
/// that wait for their agent, and which agents have a lease call of their own under way or are leaving.
#[derive(Debug, Default)]
pub struct Renewals {
    policy: Policy,
    grants: Vec<Grant>,
    /// Rows from the node's last run: an agent that joins the mesh again (taken up from the holder) gets its own back.
    pending: Vec<Row>,
    /// Agents with a lease call of their own on its way to the board: none of their grants is renewed meanwhile, so a renewal and
    /// the agent's own call never cross at the hub.
    busy: HashMap<AgentId, u32>,
    /// Agents whose endpoint is closing: nothing of theirs is renewed or recorded any more.
    leaving: HashSet<AgentId>,
    next_key: u64,
    /// What is kept on disk changed since [`Self::take_dirty`].
    dirty: bool,
}

impl Renewals {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_policy(policy: Policy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }

    pub fn set_policy(&mut self, policy: Policy) {
        self.policy = policy;
    }

    /// `agent` was granted the lease `lease_id` on `paths` for `ttl_s`, in answer to a call sent at `sent_at_ms`: by the hub's board
    /// (`on_board`, renewed) or by the node's own broker (remembered for a later `BoardJoin`). Paths the agent held under an older
    /// grant of the project now belong to this one (as in the lease table), and a grant left with none is forgotten. Returns
    /// whether it is recorded (not when its arguments are not what the table would have granted, or the agent is over its limits).
    #[allow(clippy::too_many_arguments)]
    pub fn granted(
        &mut self,
        agent: &AgentId,
        project: &str,
        lease_id: &str,
        paths: &[String],
        ttl_s: &Value,
        sent_at_ms: u64,
        on_board: bool,
    ) -> bool {
        if self.leaving.contains(agent) || !id_ok(lease_id) {
            return false;
        }
        let Some(ttl) = ttl_ms(ttl_s) else {
            return false;
        };
        let Some(paths) = paths
            .iter()
            .map(|p| canonical(p))
            .collect::<Option<Vec<_>>>()
        else {
            return false;
        };
        if paths.is_empty() {
            return false;
        }
        let before = self.grants.len();
        for g in self
            .grants
            .iter_mut()
            .filter(|g| &g.agent == agent && g.project == project)
        {
            g.paths.retain(|p| !paths.contains(p));
        }
        self.grants.retain(|g| !g.paths.is_empty());
        self.dirty |= self.grants.len() != before;
        let (count, held) = self
            .grants
            .iter()
            .filter(|g| &g.agent == agent)
            .fold((0, 0), |(n, p), g| (n + 1, p + g.paths.len()));
        if count >= self.policy.max_grants_per_agent
            || held + paths.len() > self.policy.max_paths_per_agent
        {
            return false;
        }
        self.next_key += 1;
        self.grants.push(Grant {
            key: self.next_key,
            agent: agent.clone(),
            project: project.to_string(),
            shown: lease_id.to_string(),
            current: lease_id.to_string(),
            paths,
            ttl_s: ttl_s.clone(),
            ttl_ms: ttl,
            granted_at_ms: sent_at_ms,
            on_board,
            due_now: false,
            releasing: false,
            in_flight_since: None,
            not_before_ms: 0,
        });
        self.dirty |= on_board;
        true
    }

    /// The project became shared and the node handed its unfinished local work to the board (`BoardJoin`): the remembered local
    /// grants of the project whose paths `moved` names for their holder (`(path, holder)`) are on the board now, under an id the
    /// node does not know yet, so each is renewed at once (from `now_ms`) to learn it. The agent keeps using its local id.
    pub fn moved_to_board(&mut self, project: &str, moved: &[(String, AgentId)], now_ms: u64) {
        for g in self
            .grants
            .iter_mut()
            .filter(|g| !g.on_board && g.project == project)
        {
            if g.paths
                .iter()
                .all(|p| moved.iter().any(|(m, h)| m == p && h == &g.agent))
            {
                g.on_board = true;
                g.due_now = true;
                g.not_before_ms = now_ms;
                self.dirty = true;
            }
        }
    }

    /// The board's id now for the lease `agent` knows as `lease_id`, when it differs (it was renewed); `None` to send it as it is.
    pub fn release_target(&self, agent: &AgentId, lease_id: &str) -> Option<String> {
        self.grants
            .iter()
            .find(|g| &g.agent == agent && g.shown == lease_id && g.current != lease_id)
            .map(|g| g.current.clone())
    }

    /// `agent` released the lease it knows as `lease_id` (or the board no longer has it): it is forgotten.
    pub fn released(&mut self, agent: &AgentId, lease_id: &str) {
        let before = self.grants.len();
        self.grants
            .retain(|g| !(&g.agent == agent && (g.shown == lease_id || g.current == lease_id)));
        self.dirty |= self.grants.len() != before;
    }

    /// `agent` asked for the release of `lease_id` and the answer did not say it happened: it is not renewed again (the agent
    /// wants it gone; at worst it ends at its TTL), but its id is still translated for the agent's next try.
    pub fn release_failed(&mut self, agent: &AgentId, lease_id: &str) {
        for g in self
            .grants
            .iter_mut()
            .filter(|g| &g.agent == agent && (g.shown == lease_id || g.current == lease_id))
        {
            g.releasing = true;
            self.dirty = true;
        }
    }

    /// The agent's endpoint starts closing: nothing of it is renewed or recorded from now on (closing can take a while).
    pub fn agent_leaving(&mut self, agent: &AgentId) {
        self.leaving.insert(agent.clone());
    }

    /// The agent ended (finished, stopped, gave up) or left the mesh: none of its leases is renewed again, and an answer to a
    /// renewal still on its way is ignored.
    pub fn agent_left(&mut self, agent: &AgentId) {
        let before = (self.grants.len(), self.pending.len());
        self.grants.retain(|g| &g.agent != agent);
        self.pending.retain(|r| &r.agent != agent);
        self.dirty |= before != (self.grants.len(), self.pending.len());
        self.busy.remove(agent);
        self.leaving.remove(agent);
    }

    /// `agent` of `project` joined the mesh: rows the node's last run kept for it are its grants again (an agent taken up from the
    /// holder after a node restart).
    pub fn adopt(&mut self, agent: &AgentId, project: &str) {
        let (mine, rest): (Vec<Row>, Vec<Row>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|r| &r.agent == agent && r.project == project);
        self.pending = rest;
        for r in mine {
            let Some(ttl) = ttl_ms(&r.ttl_s) else {
                continue;
            };
            self.next_key += 1;
            self.grants.push(Grant {
                key: self.next_key,
                agent: r.agent,
                project: r.project,
                shown: r.shown,
                current: r.current,
                paths: r.paths,
                ttl_s: r.ttl_s,
                ttl_ms: ttl,
                granted_at_ms: r.granted_at_ms,
                on_board: true,
                due_now: false,
                releasing: false,
                in_flight_since: None,
                not_before_ms: 0,
            });
            self.dirty = true;
        }
    }

    /// Rows read back from the last run, waiting for their agents ([`Self::adopt`]).
    pub fn restore(&mut self, rows: Vec<Row>) {
        self.pending.extend(rows);
    }

    /// Nothing is renewed any more (the node's mesh no longer reaches the hub: `mesh.remote = false`).
    pub fn clear(&mut self) {
        self.grants.clear();
        self.pending.clear();
        self.dirty = true;
    }

    /// `agent` starts a lease call of its own; its grants are not renewed until [`Self::end_call`].
    pub fn begin_call(&mut self, agent: &AgentId) {
        *self.busy.entry(agent.clone()).or_insert(0) += 1;
    }

    pub fn end_call(&mut self, agent: &AgentId) {
        if let Some(n) = self.busy.get_mut(agent) {
            *n -= 1;
            if *n == 0 {
                self.busy.remove(agent);
            }
        }
    }

    /// Whether a renewal of one of `agent`'s grants is on its way.
    pub fn in_flight(&self, agent: &AgentId) -> bool {
        self.grants
            .iter()
            .any(|g| &g.agent == agent && g.in_flight_since.is_some())
    }

    /// The renewals to send now; each is marked as on its way until [`Self::answered`]. Grants (and kept rows) whose end passed
    /// with no renewal are forgotten (the hub let them go), and a renewal left unanswered for [`IN_FLIGHT_MS`] counts as failed.
    pub fn due(&mut self, now_ms: u64) -> Vec<Renewal> {
        for g in &mut self.grants {
            if g.in_flight_since
                .is_some_and(|s| now_ms.saturating_sub(s) >= IN_FLIGHT_MS)
            {
                g.in_flight_since = None;
                g.not_before_ms = now_ms.saturating_add(RETRY_MS);
            }
        }
        let before = (self.grants.len(), self.pending.len());
        self.grants
            .retain(|g| g.in_flight_since.is_some() || now_ms < g.ends_at());
        self.pending.retain(|r| {
            ttl_ms(&r.ttl_s).is_some_and(|t| now_ms < r.granted_at_ms.saturating_add(t))
        });
        self.dirty |= before != (self.grants.len(), self.pending.len());
        let mut out = Vec::new();
        for g in &mut self.grants {
            if g.in_flight_since.is_some()
                || self.busy.contains_key(&g.agent)
                || self.leaving.contains(&g.agent)
            {
                continue;
            }
            let Some(at) = g.renew_at(&self.policy) else {
                continue;
            };
            if now_ms >= at && now_ms < g.ends_at() {
                g.in_flight_since = Some(now_ms);
                out.push(Renewal {
                    key: g.key,
                    agent: g.agent.clone(),
                    project: g.project.clone(),
                    arguments: json!({"paths": g.paths, "ttl_s": g.ttl_s}),
                    sent_at_ms: now_ms,
                });
            }
        }
        out
    }

    /// The answer to the renewal `key` sent at `sent_at_ms`. Ignored when the grant is gone (released, or its agent ended). A late
    /// answer to a renewal that was given up as unanswered counts only when it is newer than what the grant knows: a grant
    /// counts from the newest renewal the board answered, under that renewal's id (the board's id now).
    pub fn answered(&mut self, key: u64, outcome: Outcome, sent_at_ms: u64, now_ms: u64) {
        let Some(i) = self.grants.iter().position(|g| g.key == key) else {
            return;
        };
        let g = &mut self.grants[i];
        let awaited = g.in_flight_since == Some(sent_at_ms);
        if awaited {
            g.in_flight_since = None;
        }
        let newer = sent_at_ms >= g.granted_at_ms;
        match outcome {
            Outcome::Renewed { lease_id } => {
                if newer {
                    g.current = lease_id;
                    g.granted_at_ms = sent_at_ms;
                    g.due_now = false;
                    self.dirty = true;
                }
                if awaited {
                    g.not_before_ms = 0;
                }
            }
            Outcome::Retry if awaited && now_ms < g.ends_at() => {
                g.not_before_ms = now_ms.saturating_add(RETRY_MS);
            }
            Outcome::Retry if awaited => {
                self.grants.remove(i);
                self.dirty = true;
            }
            // A failure of a renewal given up on changes nothing: the one on its way now (or the next) decides.
            Outcome::Retry => {}
            Outcome::Lost if newer => {
                self.grants.remove(i);
                self.dirty = true;
            }
            Outcome::Lost => {}
        }
    }

    /// The rows to keep across a node restart: board grants that are renewed, and rows still waiting for their agent.
    pub fn rows(&self) -> Vec<Row> {
        self.grants
            .iter()
            .filter(|g| g.on_board && !g.releasing)
            .map(Grant::row)
            .chain(self.pending.iter().cloned())
            .collect()
    }

    /// Whether what is kept on disk changed since the last call (and clears it).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// What is kept on disk must be written again (the last write failed).
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// How many of `agent`'s board grants are renewed.
    pub fn renewing(&self, agent: &AgentId) -> usize {
        self.grants
            .iter()
            .filter(|g| &g.agent == agent && g.on_board && !g.releasing)
            .count()
    }

    /// How many grants are recorded (board and local).
    pub fn len(&self) -> usize {
        self.grants.len()
    }

    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }
}

/// The kept file: each row with its grant time on the wall clock (`wall_now_ms` read at the same moment as `mono_now_ms`), since
/// the monotonic clock starts again with the next run.
pub fn encode(rows: &[Row], mono_now_ms: u64, wall_now_ms: u64) -> Vec<u8> {
    let rows: Vec<Value> = rows
        .iter()
        .map(|r| {
            let age = mono_now_ms.saturating_sub(r.granted_at_ms);
            json!({
                "agent": r.agent.as_str(),
                "project": r.project,
                "shown": r.shown,
                "current": r.current,
                "paths": r.paths,
                "ttl_s": r.ttl_s,
                "granted_at_unix_ms": wall_now_ms.saturating_sub(age),
            })
        })
        .collect();
    json!({"version": VERSION, "rows": rows})
        .to_string()
        .into_bytes()
}

/// The file is not a kept-renewals file at all (it is set aside); a row that is not valid is only skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError;

/// Reads the kept file back into rows on this run's monotonic clock. A row whose grant time is after `wall_now_ms` (the wall clock
/// went back: its age cannot be known) or whose end has passed is dropped, as is one that is not valid; the rest count from their
/// wall-clock age, so none ends later than it did.
pub fn decode(bytes: &[u8], mono_now_ms: u64, wall_now_ms: u64) -> Result<Vec<Row>, FormatError> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(FormatError);
    }
    let v: Value = serde_json::from_slice(bytes).map_err(|_| FormatError)?;
    if v.get("version").and_then(Value::as_u64) != Some(VERSION) {
        return Err(FormatError);
    }
    let rows = v.get("rows").and_then(Value::as_array).ok_or(FormatError)?;
    let row = |r: &Value| -> Option<Row> {
        let s = |k: &str| r.get(k).and_then(Value::as_str).map(str::to_string);
        let (agent, project, shown, current) =
            (s("agent")?, s("project")?, s("shown")?, s("current")?);
        if agent.is_empty() || project.is_empty() || !id_ok(&shown) || !id_ok(&current) {
            return None;
        }
        let paths = r
            .get("paths")?
            .as_array()?
            .iter()
            .map(|p| p.as_str().and_then(canonical))
            .collect::<Option<Vec<_>>>()?;
        if paths.is_empty() || paths.len() > Policy::default().max_paths_per_agent {
            return None;
        }
        let ttl_s = r.get("ttl_s")?.clone();
        let ttl = ttl_ms(&ttl_s)?;
        let wall = r.get("granted_at_unix_ms")?.as_u64()?;
        let age = wall_now_ms.checked_sub(wall)?;
        if age >= ttl || age > mono_now_ms {
            return None;
        }
        Some(Row {
            agent: AgentId::new(agent),
            project,
            shown,
            current,
            paths,
            ttl_s,
            granted_at_ms: mono_now_ms - age,
        })
    };
    Ok(rows.iter().take(MAX_ROWS).filter_map(row).collect())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// The tests' policy: the node's, with a 2 s TTL floor and a 1 s gap, so the numbers stay small.
    fn short() -> Policy {
        Policy {
            min_ttl_ms: 2_000,
            min_gap_ms: 1_000,
            ..Policy::default()
        }
    }

    fn a() -> AgentId {
        AgentId::new("a")
    }

    fn grant(r: &mut Renewals, id: &str, paths: &[&str], ttl_s: Value, at: u64) -> bool {
        let paths: Vec<String> = paths.iter().map(|p| (*p).to_string()).collect();
        r.granted(&a(), "p", id, &paths, &ttl_s, at, true)
    }

    fn renewed(id: &str) -> Outcome {
        Outcome::Renewed {
            lease_id: id.into(),
        }
    }

    #[test]
    fn a_grant_is_renewed_at_half_its_ttl_and_again_from_each_renewal() {
        let mut r = Renewals::new();
        assert!(grant(&mut r, "l-1", &["src/a.rs"], json!(60), 1_000));
        assert!(r.due(30_999).is_empty());
        let due = r.due(31_000);
        assert_eq!(due.len(), 1);
        assert_eq!(
            due[0].arguments,
            json!({"paths": ["src/a.rs"], "ttl_s": 60})
        );
        assert_eq!(due[0].agent, a());
        assert_eq!(due[0].project, "p");
        // On its way: not handed out twice.
        assert!(r.due(40_000).is_empty());
        assert!(r.in_flight(&a()));
        r.answered(due[0].key, renewed("l-2"), 31_000, 31_200);
        assert!(!r.in_flight(&a()));
        // The next one counts from when the renewal was sent.
        assert!(r.due(60_999).is_empty());
        assert_eq!(r.due(61_000).len(), 1);
    }

    /// Review of #1243 (F1): a short TTL is not renewed at all, so one call cannot make the board record a grant every second.
    #[test]
    fn a_ttl_below_thirty_seconds_is_never_renewed_and_ends_as_asked() {
        let mut r = Renewals::new();
        assert!(grant(&mut r, "l-1", &["a"], json!(29.9), 0));
        for t in (0..29_900).step_by(500) {
            assert!(r.due(t).is_empty(), "renewed at {t}");
        }
        assert!(r.due(29_900).is_empty());
        assert!(r.is_empty(), "an ended grant is forgotten");
        // Exactly thirty seconds is renewed, every fifteen.
        assert!(grant(&mut r, "l-2", &["b"], json!(30), 0));
        assert!(r.due(14_999).is_empty());
        assert_eq!(r.due(15_000).len(), 1);
    }

    /// Review of #1243 (F1): an agent has at most 256 paths renewed over all its grants; a grant that would go over is not
    /// renewed (it ends at its TTL), and paths taken again count once.
    #[test]
    fn an_agent_has_at_most_256_paths_and_64_grants_renewed() {
        let mut r = Renewals::new();
        let many: Vec<String> = (0..256).map(|i| format!("f{i}")).collect();
        assert!(r.granted(&a(), "p", "l-1", &many, &json!(60), 0, true));
        assert!(!grant(&mut r, "l-2", &["g"], json!(60), 0), "257 paths");
        // Taking one of its paths again is no new path.
        assert!(grant(&mut r, "l-3", &["f0"], json!(60), 0));
        let too_many: Vec<String> = (0..257).map(|i| format!("h{i}")).collect();
        assert!(!r.granted(
            &AgentId::new("b"),
            "p",
            "l-4",
            &too_many,
            &json!(60),
            0,
            true
        ));
        let mut r = Renewals::new();
        for i in 0..64 {
            assert!(grant(
                &mut r,
                &format!("l-{i}"),
                &[&format!("f{i}")],
                json!(60),
                0
            ));
        }
        assert!(!grant(&mut r, "l-x", &["g"], json!(60), 0));
        assert!(r.granted(
            &AgentId::new("b"),
            "p",
            "l-y",
            &["g".into()],
            &json!(60),
            0,
            true
        ));
    }

    #[test]
    fn the_gap_never_falls_below_the_policys() {
        let mut r = Renewals::with_policy(Policy {
            min_ttl_ms: 1_000,
            min_gap_ms: 5_000,
            ..Policy::default()
        });
        assert!(grant(&mut r, "l-1", &["a"], json!(8), 0));
        assert!(r.due(4_999).is_empty());
        assert_eq!(r.due(5_000).len(), 1);
    }

    #[test]
    fn a_huge_or_odd_ttl_saturates_instead_of_wrapping() {
        let mut r = Renewals::new();
        assert!(grant(&mut r, "l-1", &["a"], json!(1e300), u64::MAX - 10));
        assert!(r.due(u64::MAX - 1).is_empty());
        assert_eq!(r.len(), 1);
        assert!(!grant(&mut r, "l-2", &["b"], json!(-1), 0));
        assert!(!grant(&mut r, "l-3", &["b"], json!("60"), 0));
        assert!(!grant(&mut r, "l-4", &["b"], Value::Null, 0));
        assert_eq!(ttl_ms(&json!(1.5)), Some(1_500));
        assert_eq!(ttl_ms(&json!(f64::MAX)), Some(u64::MAX));
    }

    #[test]
    fn a_failed_renewal_is_tried_again_until_the_grants_end_then_forgotten() {
        let mut r = Renewals::with_policy(short());
        grant(&mut r, "l-1", &["a"], json!(10), 0);
        let d = r.due(5_000);
        r.answered(d[0].key, Outcome::Retry, 5_000, 5_100);
        assert!(r.due(6_099).is_empty(), "waits a second");
        let d = r.due(6_100);
        assert_eq!(d.len(), 1);
        r.answered(d[0].key, Outcome::Retry, 6_100, 9_500);
        assert!(r.due(10_499).is_empty(), "the grant ended at 10 s");
        assert!(r.is_empty(), "never taken back after its end");
        // A failure answered after the end forgets it too.
        grant(&mut r, "l-2", &["a"], json!(10), 0);
        let d = r.due(5_000);
        r.answered(d[0].key, Outcome::Retry, 5_000, 10_000);
        assert!(r.is_empty());
    }

    #[test]
    fn a_renewal_on_its_way_at_the_end_may_still_succeed() {
        let mut r = Renewals::with_policy(short());
        grant(&mut r, "l-1", &["a"], json!(4), 0);
        let d = r.due(2_000);
        // The hub answers late, after the node's idea of the end: the hub received it in time and granted it.
        assert!(r.due(4_500).is_empty());
        assert_eq!(r.len(), 1);
        r.answered(d[0].key, renewed("l-2"), 2_000, 4_600);
        assert_eq!(r.release_target(&a(), "l-1").as_deref(), Some("l-2"));
        assert_eq!(r.due(4_000).len(), 1);
    }

    #[test]
    fn a_refused_renewal_ends_the_grant_at_once() {
        let mut r = Renewals::with_policy(short());
        grant(&mut r, "l-1", &["a"], json!(10), 0);
        let d = r.due(5_000);
        r.answered(d[0].key, Outcome::Lost, 5_000, 5_100);
        assert!(r.is_empty());
        assert!(r.due(6_000).is_empty());
    }

    #[test]
    fn an_unanswered_renewal_counts_as_failed_after_its_deadline() {
        let mut r = Renewals::new();
        grant(&mut r, "l-1", &["a"], json!(600), 0);
        let first = r.due(300_000);
        assert_eq!(first.len(), 1);
        assert!(r.due(300_000 + IN_FLIGHT_MS - 1).is_empty());
        assert!(
            r.due(300_000 + IN_FLIGHT_MS).is_empty(),
            "waits the retry gap"
        );
        let second = r.due(300_000 + IN_FLIGHT_MS + RETRY_MS);
        assert_eq!(second.len(), 1);
        // The late answer to the first is newer than what the grant knows: the board's id now. The second is still on its way.
        r.answered(first[0].key, renewed("l-9"), first[0].sent_at_ms, 400_000);
        assert_eq!(r.release_target(&a(), "l-1").as_deref(), Some("l-9"));
        assert!(r.in_flight(&a()));
        r.answered(
            second[0].key,
            renewed("l-10"),
            second[0].sent_at_ms,
            400_001,
        );
        assert_eq!(r.release_target(&a(), "l-1").as_deref(), Some("l-10"));
        assert!(!r.in_flight(&a()));
    }

    #[test]
    fn a_late_answer_older_than_the_grant_changes_nothing() {
        let mut r = Renewals::new();
        grant(&mut r, "l-1", &["a"], json!(600), 0);
        let first = r.due(300_000);
        // Given up as unanswered, then sent again.
        assert!(r.due(300_000 + IN_FLIGHT_MS).is_empty());
        let second = r.due(300_000 + IN_FLIGHT_MS + RETRY_MS);
        r.answered(
            second[0].key,
            renewed("l-10"),
            second[0].sent_at_ms,
            400_000,
        );
        // The given-up first renewal answers after the second: an id the board already replaced, or a failure, is ignored.
        r.answered(first[0].key, renewed("l-9"), first[0].sent_at_ms, 400_001);
        assert_eq!(r.release_target(&a(), "l-1").as_deref(), Some("l-10"));
        r.answered(first[0].key, Outcome::Lost, first[0].sent_at_ms, 400_002);
        r.answered(first[0].key, Outcome::Retry, first[0].sent_at_ms, 400_003);
        assert_eq!(r.len(), 1);
        assert!(
            r.due(400_003).is_empty(),
            "no retry gap from a stale failure"
        );
        // The grant now counts from the second renewal.
        assert_eq!(r.due(second[0].sent_at_ms + 300_000).len(), 1);
    }

    #[test]
    fn the_agents_release_names_the_boards_current_id() {
        let mut r = Renewals::with_policy(short());
        grant(&mut r, "l-1", &["a"], json!(10), 0);
        assert_eq!(r.release_target(&a(), "l-1"), None, "not renewed yet");
        let d = r.due(5_000);
        r.answered(d[0].key, renewed("l-7"), 5_000, 5_010);
        assert_eq!(r.release_target(&a(), "l-1").as_deref(), Some("l-7"));
        assert_eq!(
            r.release_target(&AgentId::new("b"), "l-1"),
            None,
            "another agent's id"
        );
        r.released(&a(), "l-1");
        assert!(r.is_empty());
        // Either id releases it.
        grant(&mut r, "l-8", &["a"], json!(10), 0);
        let d = r.due(5_000);
        r.answered(d[0].key, renewed("l-9"), 5_000, 5_010);
        r.released(&a(), "l-9");
        assert!(r.is_empty());
    }

    /// Review of #1243: a release the hub could not decide keeps the id (the agent's next try reaches the board's lease) but is not
    /// renewed again.
    #[test]
    fn a_release_that_was_not_decided_keeps_the_id_and_stops_renewing() {
        let mut r = Renewals::with_policy(short());
        grant(&mut r, "l-1", &["a"], json!(10), 0);
        let d = r.due(5_000);
        r.answered(d[0].key, renewed("l-2"), 5_000, 5_010);
        r.release_failed(&a(), "l-1");
        assert_eq!(r.release_target(&a(), "l-1").as_deref(), Some("l-2"));
        assert!(r.due(10_000).is_empty());
        assert_eq!(r.renewing(&a()), 0);
        assert!(r.rows().is_empty(), "not kept across a restart");
        r.released(&a(), "l-1");
        assert!(r.is_empty());
    }

    #[test]
    fn paths_taken_again_leave_the_older_grant_as_in_the_table() {
        let mut r = Renewals::with_policy(short());
        grant(&mut r, "l-1", &["src/a.rs", "src/b.rs"], json!(10), 0);
        // The same path spelled differently is the same lease to the table.
        grant(&mut r, "l-2", &["./src//a.rs"], json!(10), 1_000);
        let d = r.due(5_000);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].arguments["paths"], json!(["src/b.rs"]));
        r.answered(d[0].key, renewed("l-3"), 5_000, 5_000);
        let d = r.due(6_000);
        assert_eq!(d[0].arguments["paths"], json!(["src/a.rs"]));
        // Taking every path again forgets the older grant.
        grant(&mut r, "l-4", &["src/b.rs", "src/a.rs"], json!(10), 7_000);
        assert_eq!(r.len(), 1);
        // Another project's and another agent's grants are untouched.
        r.granted(
            &a(),
            "q",
            "l-5",
            &["src/a.rs".into()],
            &json!(10),
            7_000,
            true,
        );
        r.granted(
            &AgentId::new("b"),
            "p",
            "l-6",
            &["x".into()],
            &json!(10),
            7_000,
            true,
        );
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn a_path_the_table_would_refuse_is_not_recorded() {
        let mut r = Renewals::new();
        assert!(!grant(&mut r, "l-1", &["../etc"], json!(60), 0));
        assert!(!grant(&mut r, "l-1", &[], json!(60), 0));
        assert!(!grant(&mut r, "", &["a"], json!(60), 0));
        assert!(!grant(
            &mut r,
            &"x".repeat(MAX_ID_BYTES + 1),
            &["a"],
            json!(60),
            0
        ));
        assert!(r.is_empty());
    }

    #[test]
    fn an_agent_that_ended_is_never_renewed_and_a_late_answer_is_ignored() {
        let mut r = Renewals::with_policy(short());
        grant(&mut r, "l-1", &["a"], json!(10), 0);
        let d = r.due(5_000);
        r.agent_left(&a());
        assert!(r.is_empty());
        // A new run of an agent with the same id takes a lease: the old renewal's answer does not touch it.
        grant(&mut r, "l-2", &["a"], json!(10), 5_500);
        r.answered(d[0].key, renewed("l-3"), 5_000, 5_600);
        assert_eq!(r.release_target(&a(), "l-2"), None);
        assert!(r.due(9_000).is_empty());
        assert_eq!(r.due(10_500).len(), 1);
    }

    /// Review of #1243: while an agent's endpoint closes (which can take a while), nothing of it is renewed or recorded.
    #[test]
    fn an_agent_that_is_leaving_is_neither_renewed_nor_recorded() {
        let mut r = Renewals::with_policy(short());
        grant(&mut r, "l-1", &["a"], json!(10), 0);
        r.agent_leaving(&a());
        assert!(r.due(5_000).is_empty());
        assert!(!grant(&mut r, "l-2", &["b"], json!(10), 5_000));
        r.agent_left(&a());
        assert!(r.is_empty());
        assert!(
            grant(&mut r, "l-3", &["b"], json!(10), 6_000),
            "a later run joins again"
        );
    }

    #[test]
    fn an_agent_with_its_own_lease_call_under_way_is_not_renewed_meanwhile() {
        let mut r = Renewals::with_policy(short());
        grant(&mut r, "l-1", &["a"], json!(10), 0);
        r.begin_call(&a());
        r.begin_call(&a());
        assert!(r.due(5_000).is_empty());
        r.end_call(&a());
        assert!(r.due(5_000).is_empty());
        r.end_call(&a());
        assert_eq!(r.due(5_000).len(), 1);
        // More ends than begins never underflow.
        r.end_call(&a());
    }

    #[test]
    fn clear_stops_every_renewal() {
        let mut r = Renewals::with_policy(short());
        grant(&mut r, "l-1", &["a"], json!(10), 0);
        r.clear();
        assert!(r.due(5_000).is_empty());
    }

    /// Review of #1243 (F2): a lease the node's own broker granted is not renewed; once the project is shared and the node handed
    /// it over, it is renewed at once (to learn the board's id) and the agent's local id still releases it.
    #[test]
    fn a_local_lease_handed_to_the_board_is_renewed_from_then_on() {
        let mut r = Renewals::with_policy(short());
        assert!(r.granted(&a(), "p", "l-5", &["src/a.rs".into()], &json!(60), 0, false));
        assert!(r.due(50_000).is_empty(), "local leases are not renewed");
        assert!(r.rows().is_empty(), "nor kept");
        // Another project's handover, or one that did not take this agent's path, changes nothing.
        r.moved_to_board("q", &[("src/a.rs".into(), a())], 1_000);
        r.moved_to_board("p", &[("src/a.rs".into(), AgentId::new("b"))], 1_000);
        assert!(r.due(1_000).is_empty());
        r.moved_to_board("p", &[("src/a.rs".into(), a())], 1_000);
        let d = r.due(1_000);
        assert_eq!(d.len(), 1, "renewed at once");
        assert_eq!(d[0].arguments, json!({"paths": ["src/a.rs"], "ttl_s": 60}));
        r.answered(d[0].key, renewed("l-77"), 1_000, 1_010);
        assert_eq!(r.release_target(&a(), "l-5").as_deref(), Some("l-77"));
        assert!(r.due(30_999).is_empty(), "then on the usual schedule");
        assert_eq!(r.due(31_000).len(), 1);
        assert_eq!(r.rows().len(), 1);
    }

    /// Review of #1243 (F2): rows kept by the last run wait for their agent; the agent that joins again gets them back on its
    /// clock, and ones whose end passed meanwhile are dropped.
    #[test]
    fn kept_rows_come_back_for_their_agent_only() {
        let mut first = Renewals::with_policy(short());
        grant(&mut first, "l-1", &["a"], json!(60), 1_000_000);
        let d = first.due(1_030_000);
        first.answered(d[0].key, renewed("l-2"), 1_030_000, 1_030_010);
        let bytes = encode(&first.rows(), 1_040_000, 5_000_000);
        // The next run: its monotonic clock starts elsewhere; 5 s of wall time went by.
        let rows = decode(&bytes, 7_000_000, 5_005_000).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].granted_at_ms, 7_000_000 - 15_000);
        let mut next = Renewals::with_policy(short());
        next.restore(rows);
        assert!(next.is_empty());
        next.adopt(&AgentId::new("b"), "p");
        next.adopt(&a(), "q");
        assert!(next.is_empty(), "another agent or project");
        next.adopt(&a(), "p");
        assert_eq!(next.release_target(&a(), "l-1").as_deref(), Some("l-2"));
        assert!(next.due(7_000_000 + 14_999).is_empty());
        assert_eq!(next.due(7_000_000 + 15_000).len(), 1);
        // Rows nobody takes up end with their lease.
        let mut idle = Renewals::with_policy(short());
        idle.restore(decode(&bytes, 7_000_000, 5_005_000).unwrap());
        idle.due(7_000_000 + 45_000);
        assert!(idle.rows().is_empty());
    }

    #[test]
    fn a_kept_file_is_read_with_care() {
        let row = Row {
            agent: a(),
            project: "p".into(),
            shown: "l-1".into(),
            current: "l-2".into(),
            paths: vec!["a".into()],
            ttl_s: json!(60),
            granted_at_ms: 100_000,
        };
        let ok = encode(std::slice::from_ref(&row), 110_000, 1_000_000);
        // 10 s old at writing; read 1 s later on a new clock.
        assert_eq!(
            decode(&ok, 50_000, 1_001_000).unwrap()[0].granted_at_ms,
            50_000 - 11_000
        );
        // The wall clock went back: the age is not known, so the row is dropped (its lease ends at its TTL).
        assert!(decode(&ok, 50_000, 980_000).unwrap().is_empty());
        // Its end passed while the node was down.
        assert!(decode(&ok, 50_000, 1_051_000).unwrap().is_empty());
        // Format errors: the file is set aside.
        for bad in [
            b"not json".to_vec(),
            json!({"version": 2, "rows": []}).to_string().into_bytes(),
            json!({"version": 1}).to_string().into_bytes(),
            vec![b' '; (MAX_FILE_BYTES + 1) as usize],
        ] {
            assert_eq!(decode(&bad, 0, 0), Err(FormatError));
        }
        // A row that is not valid is skipped; the others are kept.
        let mixed = json!({"version": 1, "rows": [
            {"agent": "a", "project": "p", "shown": "l-1", "current": "l-2", "paths": ["../x"], "ttl_s": 60,
             "granted_at_unix_ms": 1_000_000},
            {"agent": "a", "project": "p", "shown": "", "current": "l-2", "paths": ["x"], "ttl_s": 60,
             "granted_at_unix_ms": 1_000_000},
            {"agent": "a", "project": "p", "shown": "l-1", "current": "l-2", "paths": ["x"], "ttl_s": "60",
             "granted_at_unix_ms": 1_000_000},
            {"agent": "a", "project": "p", "shown": "l-1", "current": "l-2", "paths": ["x"], "ttl_s": 60,
             "granted_at_unix_ms": 1_000_000}
        ]})
        .to_string();
        assert_eq!(
            decode(mixed.as_bytes(), 50_000, 1_000_500).unwrap().len(),
            1
        );
    }

    fn tool(text: Value, is_error: bool) -> String {
        json!({"content": [{"type": "text", "text": text.to_string()}], "isError": is_error})
            .to_string()
    }

    #[test]
    fn the_boards_answer_is_read_as_renewed_lost_or_retry() {
        let ok = tool(json!({"lease_id": "l-4", "expires_at": 1}), false);
        assert_eq!(classify(Ok((true, &ok))), renewed("l-4"));
        assert_eq!(classify(Ok((false, &ok))), Outcome::Lost, "not shared");
        assert_eq!(classify(Err("node.offline")), Outcome::Retry);
        assert_eq!(classify(Err("node.timeout")), Outcome::Retry);
        for code in [
            "board.unavailable",
            "node.offline",
            "internal",
            "mesh.rate_limited",
        ] {
            let e = tool(json!({"error": {"code": code, "params": {}}}), true);
            assert_eq!(classify(Ok((true, &e))), Outcome::Retry, "{code}");
        }
        // `agent.not_found`: the board no longer counts the agent as running, and has given its leases back.
        for code in [
            "lease.held",
            "lease.invalid_path",
            "agent.not_found",
            "protocol.invalid_message",
            "x",
        ] {
            let e = tool(json!({"error": {"code": code, "params": {}}}), true);
            assert_eq!(classify(Ok((true, &e))), Outcome::Lost, "{code}");
        }
        for bad in [
            "not json".to_string(),
            json!({"content": []}).to_string(),
            tool(json!({"lease_id": 5}), false),
            tool(json!({"lease_id": ""}), false),
            tool(json!({"lease_id": "x".repeat(MAX_ID_BYTES + 1)}), false),
        ] {
            assert_eq!(classify(Ok((true, &bad))), Outcome::Lost, "{bad}");
        }
    }

    #[test]
    fn a_release_is_settled_only_when_the_board_says_so() {
        let ok = tool(json!({"ok": true}), false);
        assert!(release_settled(Some((true, &ok))));
        let gone = tool(
            json!({"error": {"code": "lease.not_found", "params": {}}}),
            true,
        );
        assert!(release_settled(Some((true, &gone))));
        let busy = tool(
            json!({"error": {"code": "board.unavailable", "params": {}}}),
            true,
        );
        assert!(!release_settled(Some((true, &busy))));
        assert!(!release_settled(None), "no answer");
        assert!(
            !release_settled(Some((false, &ok))),
            "not shared: the node's broker decides"
        );
        assert!(!release_settled(Some((true, "garbage"))));
    }
}
