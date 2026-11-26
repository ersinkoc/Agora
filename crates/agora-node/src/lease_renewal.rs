//! ADR-0039: the node renews the shared-board leases of its agents while they run, so a lease on a shared project lasts until its
//! agent releases it or ends, and its TTL only bounds how long it outlives a machine that went away (the hub ends it when the TTL
//! passes with no renewal). Leases on the node's own broker (a project only this machine has) are not renewed: ADR-0039 leaves
//! local-only projects untouched.
//!
//! This is the bookkeeping only: plain data and an injected clock (`now_ms`), no I/O, no locks. The mesh server records what the
//! hub's board granted an agent, takes the renewals that are [`Renewals::due`] from its ticker, sends each as the agent's own
//! `lease_acquire` on the board (the only lease call the board has, so no new protocol message), and gives the answer back
//! ([`Renewals::answered`]).
//!
//! A renewal is the board's `lease_acquire`, which gives the paths a new lease id. The agent keeps the id it was given: the node
//! remembers, per grant, the id the agent knows and the one the board has now, and puts the current one into the agent's
//! `lease_release` ([`Renewals::release_target`]).
//!
//! Schedule: a grant is renewed when half its TTL has passed since the call that last granted it was sent (the node's own clock, so
//! the node's idea of the end is never later than the hub's). A TTL below two seconds is not renewed (it ends as asked; renewing it
//! faster than once a second would flood the board, which records each grant). A renewal that fails for a reason that may pass (the
//! hub cannot be reached, it timed out, the board is taking over a project's work) is tried again every second until the grant's end
//! as the node reckons it; after that the lease is gone (a node that was offline does not take it back on reconnect). One that the
//! board refuses (another agent holds the path, or the board no longer has the project) ends the grant at once.

use std::collections::HashMap;

use agora_core::{AgentId, LeaseTable};
use serde_json::{json, Value};

/// No grant is renewed more often than this; a TTL shorter than twice this is not renewed at all.
pub const MIN_RENEW_GAP_MS: u64 = 1_000;
/// How long after a renewal that may succeed later failed it is tried again.
pub const RETRY_MS: u64 = 1_000;
/// A renewal with no answer after this long is taken as failed (longer than the uplink's 40 s deadline for a post, which answers
/// first, so this only covers a link that never answers).
pub const IN_FLIGHT_MS: u64 = 45_000;
/// At most this many grants of one agent are renewed; a further grant ends at its TTL, as it would without renewal.
pub const MAX_GRANTS_PER_AGENT: usize = 64;
/// A lease id longer than this from the board is not recorded (ids are short; the answer is data from another machine).
const MAX_ID_BYTES: usize = 128;

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

#[derive(Debug, Clone)]
struct Grant {
    key: u64,
    agent: AgentId,
    project: String,
    /// The lease id the agent was given and still uses.
    shown: String,
    /// The board's id for these paths now (changes on every renewal).
    current: String,
    /// As the board keeps them (normalized); the paths the agent took again under a newer grant are no longer here.
    paths: Vec<String>,
    /// The agent's own `ttl_s`, sent again unchanged.
    ttl_s: Value,
    ttl_ms: u64,
    /// When the call that last granted it was sent.
    granted_at_ms: u64,
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

    /// When to renew it next; `None` when its TTL is too short to be renewed.
    fn renew_at(&self) -> Option<u64> {
        let half = self.ttl_ms / 2;
        (half >= MIN_RENEW_GAP_MS).then(|| {
            self.granted_at_ms
                .saturating_add(half)
                .max(self.not_before_ms)
        })
    }
}

/// The seconds of a `ttl_s` argument in milliseconds, read as the broker reads it (finite, not negative); `None` otherwise.
pub fn ttl_ms(ttl_s: &Value) -> Option<u64> {
    let n = ttl_s.as_f64()?;
    // `as` saturates: a huge TTL is the longest one, never a wrapped short one.
    (n.is_finite() && n >= 0.0).then_some((n * 1000.0) as u64)
}

/// A path as the board's lease table keeps it (`./src//a.rs` is `src/a.rs`), with the table's own rule; `None` when the table
/// would refuse it.
fn canonical(path: &str) -> Option<String> {
    LeaseTable::new()
        .acquire(path, &AgentId::new("-"), 0, 1)
        .ok()
        .map(|l| l.path)
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
    let Ok(result) = serde_json::from_str::<Value>(result_json) else {
        return Outcome::Lost;
    };
    let Some(text) = result
        .get("content")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("text"))
        .and_then(Value::as_str)
    else {
        return Outcome::Lost;
    };
    let Ok(body) = serde_json::from_str::<Value>(text) else {
        return Outcome::Lost;
    };
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        // Refusals that change nothing and may pass: the hub's board is busy or cannot store, or the call could not be sent.
        let code = body
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str);
        return match code {
            Some(
                "board.unavailable" | "node.offline" | "node.timeout" | "internal"
                | "mesh.unavailable" | "mesh.rate_limited",
            ) => Outcome::Retry,
            _ => Outcome::Lost,
        };
    }
    match body.get("lease_id").and_then(Value::as_str) {
        Some(id) if !id.is_empty() && id.len() <= MAX_ID_BYTES => Outcome::Renewed {
            lease_id: id.to_string(),
        },
        _ => Outcome::Lost,
    }
}

/// The board grants the node renews, and which agents have a lease call of their own under way.
#[derive(Debug, Default)]
pub struct Renewals {
    grants: Vec<Grant>,
    /// Agents with a lease call of their own on its way to the board: none of their grants is renewed meanwhile, so a renewal and
    /// the agent's own call never cross at the hub.
    busy: HashMap<AgentId, u32>,
    next_key: u64,
}

impl Renewals {
    pub fn new() -> Self {
        Self::default()
    }

    /// The board granted `agent` the lease `lease_id` on `paths` for `ttl_s`, in answer to a call sent at `sent_at_ms`. Paths the
    /// agent held under an older grant now belong to this one (as on the board), and a grant left with none is forgotten. Returns
    /// whether it is renewed (not when its arguments are not what the board would have granted, or the agent has too many).
    pub fn granted(
        &mut self,
        agent: &AgentId,
        project: &str,
        lease_id: &str,
        paths: &[String],
        ttl_s: &Value,
        sent_at_ms: u64,
    ) -> bool {
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
        if paths.is_empty() || lease_id.is_empty() || lease_id.len() > MAX_ID_BYTES {
            return false;
        }
        for g in self
            .grants
            .iter_mut()
            .filter(|g| &g.agent == agent && g.project == project)
        {
            g.paths.retain(|p| !paths.contains(p));
        }
        self.grants.retain(|g| !g.paths.is_empty());
        if self.grants.iter().filter(|g| &g.agent == agent).count() >= MAX_GRANTS_PER_AGENT {
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
            in_flight_since: None,
            not_before_ms: 0,
        });
        true
    }

    /// The board's id now for the lease `agent` knows as `lease_id`, when it differs (it was renewed); `None` to send it as it is.
    pub fn release_target(&self, agent: &AgentId, lease_id: &str) -> Option<String> {
        self.grants
            .iter()
            .find(|g| &g.agent == agent && g.shown == lease_id && g.current != lease_id)
            .map(|g| g.current.clone())
    }

    /// `agent` released the lease it knows as `lease_id` (or the board no longer has it): it is not renewed again.
    pub fn released(&mut self, agent: &AgentId, lease_id: &str) {
        self.grants
            .retain(|g| !(&g.agent == agent && g.shown == lease_id));
    }

    /// The agent ended (finished, stopped, crashed, gave up) or left the mesh: none of its leases is renewed again, and an answer to a
    /// renewal still on its way is ignored.
    pub fn agent_left(&mut self, agent: &AgentId) {
        self.grants.retain(|g| &g.agent != agent);
        self.busy.remove(agent);
    }

    /// Nothing is renewed any more (the node's mesh no longer reaches the hub: `mesh.remote = false`).
    pub fn clear(&mut self) {
        self.grants.clear();
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

    /// The renewals to send now; each is marked as on its way until [`Self::answered`]. Grants whose end passed with no renewal
    /// are forgotten (the hub let them go), and a renewal left unanswered for [`IN_FLIGHT_MS`] counts as failed.
    pub fn due(&mut self, now_ms: u64) -> Vec<Renewal> {
        for g in &mut self.grants {
            if g.in_flight_since
                .is_some_and(|s| now_ms.saturating_sub(s) >= IN_FLIGHT_MS)
            {
                g.in_flight_since = None;
                g.not_before_ms = now_ms.saturating_add(RETRY_MS);
            }
        }
        self.grants
            .retain(|g| g.in_flight_since.is_some() || now_ms < g.ends_at());
        let mut out = Vec::new();
        for g in &mut self.grants {
            if g.in_flight_since.is_some() || self.busy.contains_key(&g.agent) {
                continue;
            }
            let Some(at) = g.renew_at() else {
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
            }
            // A failure of a renewal given up on changes nothing: the one on its way now (or the next) decides.
            Outcome::Retry => {}
            Outcome::Lost if newer => {
                self.grants.remove(i);
            }
            Outcome::Lost => {}
        }
    }

    /// How many grants are renewed (for tests and the node's view).
    pub fn len(&self) -> usize {
        self.grants.len()
    }

    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn a() -> AgentId {
        AgentId::new("a")
    }

    fn grant(r: &mut Renewals, id: &str, paths: &[&str], ttl_s: Value, at: u64) -> bool {
        let paths: Vec<String> = paths.iter().map(|p| (*p).to_string()).collect();
        r.granted(&a(), "p", id, &paths, &ttl_s, at)
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

    #[test]
    fn a_ttl_below_twice_the_floor_is_never_renewed_and_ends_as_asked() {
        let mut r = Renewals::new();
        assert!(grant(&mut r, "l-1", &["a"], json!(1.9), 0));
        for t in (0..1_900).step_by(100) {
            assert!(r.due(t).is_empty(), "renewed at {t}");
        }
        assert!(r.due(1_900).is_empty());
        assert!(r.is_empty(), "an ended grant is forgotten");
        // Exactly twice the floor is renewed, once a second.
        assert!(grant(&mut r, "l-2", &["b"], json!(2), 0));
        assert_eq!(r.due(1_000).len(), 1);
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
        let mut r = Renewals::new();
        grant(&mut r, "l-1", &["a"], json!(10), 0);
        let d = r.due(5_000);
        r.answered(d[0].key, Outcome::Retry, 5_000, 5_100);
        assert!(r.due(6_099).is_empty(), "waits a second");
        let d = r.due(6_100);
        assert_eq!(d.len(), 1);
        r.answered(d[0].key, Outcome::Retry, 6_100, 9_500);
        let d = r.due(10_499);
        assert!(d.is_empty(), "the grant ended at 10 s");
        assert!(r.is_empty(), "never taken back after its end");
        // A failure answered after the end forgets it too.
        grant(&mut r, "l-2", &["a"], json!(10), 0);
        let d = r.due(5_000);
        r.answered(d[0].key, Outcome::Retry, 5_000, 10_000);
        assert!(r.is_empty());
    }

    #[test]
    fn a_renewal_on_its_way_at_the_end_may_still_succeed() {
        let mut r = Renewals::new();
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
        let mut r = Renewals::new();
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
        let mut r = Renewals::new();
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
    }

    #[test]
    fn paths_taken_again_leave_the_older_grant_as_on_the_board() {
        let mut r = Renewals::new();
        grant(&mut r, "l-1", &["src/a.rs", "src/b.rs"], json!(10), 0);
        // The same path spelled differently is the same lease to the board.
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
        r.granted(&a(), "q", "l-5", &["src/a.rs".into()], &json!(10), 7_000);
        r.granted(
            &AgentId::new("b"),
            "p",
            "l-6",
            &["x".into()],
            &json!(10),
            7_000,
        );
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn a_path_the_board_would_refuse_is_not_recorded() {
        let mut r = Renewals::new();
        assert!(!grant(&mut r, "l-1", &["../etc"], json!(10), 0));
        assert!(!grant(&mut r, "l-1", &[], json!(10), 0));
        assert!(!grant(&mut r, "", &["a"], json!(10), 0));
        assert!(!grant(
            &mut r,
            &"x".repeat(MAX_ID_BYTES + 1),
            &["a"],
            json!(10),
            0
        ));
        assert!(r.is_empty());
    }

    #[test]
    fn an_agent_that_ended_is_never_renewed_and_a_late_answer_is_ignored() {
        let mut r = Renewals::new();
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

    #[test]
    fn an_agent_with_its_own_lease_call_under_way_is_not_renewed_meanwhile() {
        let mut r = Renewals::new();
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
    fn an_agent_has_at_most_so_many_renewed_grants() {
        let mut r = Renewals::new();
        for i in 0..MAX_GRANTS_PER_AGENT {
            assert!(grant(
                &mut r,
                &format!("l-{i}"),
                &[&format!("f{i}")],
                json!(10),
                0
            ));
        }
        assert!(!grant(&mut r, "l-x", &["g"], json!(10), 0));
        assert!(r.granted(&AgentId::new("b"), "p", "l-y", &["g".into()], &json!(10), 0));
    }

    #[test]
    fn clear_stops_every_renewal() {
        let mut r = Renewals::new();
        grant(&mut r, "l-1", &["a"], json!(10), 0);
        r.clear();
        assert!(r.due(5_000).is_empty());
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
        for code in [
            "lease.held",
            "lease.invalid_path",
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
}
