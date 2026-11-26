//! The hub: one place that owns the store and the live view, takes events from node sessions, and fans them
//! out to subscribers (clients). No sockets and no framing: the server layer authenticates a peer, runs a
//! [`NodeSession`] for it and hands payloads here; a client connection calls [`Hub::subscribe`].
//!
//! Guarantees:
//! - An event is applied to the store, folded into the view and offered to subscribers as one step under one lock,
//!   so a subscription that takes its snapshot (or backlog) and registers for live events in that same step sees
//!   every event exactly once, in order, however many nodes are ingesting at the same time.
//! - A subscriber that cannot keep up is dropped, not waited for: ingestion never blocks on a slow client. It
//!   reconnects with `after = last position seen` and gets the backlog (or a fresh snapshot if too far behind).
//! - Facts only the hub can know (a node's connection dropped) are recorded as events from producer `hub`, in the
//!   same log, so every client learns them the same way and a rebuilt view agrees with the live one.

use std::ops::Deref;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};

use agora_protocol::v1::event::Body;
use agora_protocol::v1::{Event, NodeLink, NodeLinkReason, NodeOffline};
use prost::Message;

use crate::ca::RESERVED_HUB_ID;
use crate::projection::View;
use crate::session::{Ingest, NodeSession, SessionError};
use crate::store::{Append, EventStore, SqliteStore, StoreError};

/// Events a subscriber may have queued before it is considered too slow and dropped.
pub const SUBSCRIBER_BUFFER: usize = 1024;
const PAGE: usize = 1000;
/// A subscriber that resumes from a position replays at most this many events, or this many bytes of them; further behind, it gets
/// a fresh snapshot instead (architecture review R3). Replay runs under the hub lock, so its cost must not grow with how long a
/// client was away: every node's ingest and every other client waits while it runs.
pub const MAX_REPLAY_EVENTS: u64 = 10_000;
pub const MAX_REPLAY_BYTES: usize = 8 << 20;

/// One event with its place in the global log.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivered {
    pub pos: u64,
    pub event: Event,
}

/// What a new subscriber receives.
pub struct Subscription {
    /// Present when the subscriber asked from the start, or asked from a position the hub cannot serve.
    pub snapshot: Option<(View, u64)>,
    /// Events after the requested position, oldest first (empty with a snapshot).
    pub backlog: Vec<Delivered>,
    /// Everything after the snapshot/backlog, live. Closed when the subscriber falls behind.
    pub live: Receiver<Delivered>,
}

struct Inner {
    view: View,
    pos: u64,
    hub_seq: u64,
    subs: Vec<SyncSender<Delivered>>,
    /// Subscribers let go because their queue was full (a slow window) or closed (`GET /api/metrics`).
    subs_dropped: u64,
    /// How long an agent waited for a person's answer (`GET /api/metrics`).
    approval_wait: crate::metrics::Histogram,
    /// #996: presence facts about connections that ended while the store refused the write, oldest first, with the hub clock of
    /// the end. They are written before any later hub fact (so a node's next `NodeLink { up }` always comes after them) and by
    /// [`Hub::record_unrecorded`] once the store works again. Memory only: a hub that restarts records every node offline anyway.
    unrecorded: std::collections::VecDeque<(Body, u64)>,
    /// #1036: nodes a published fact took offline since board membership last looked ([`Hub::board_membership`]). Membership
    /// compares the online nodes with its last look, so a node that left and came back between two looks (its late end and its
    /// redial's `up` written together, #996) would otherwise look as if it never left.
    left: std::collections::BTreeSet<String>,
    /// ADR-0112 Amendment A: the machines whose latest link agreed on `mesh.fleet_status.v1`; only they and their agents are in the
    /// hub-wide view. Kept when the link ends (an offline machine is decided by its last link), memory only: after a hub restart a
    /// machine is left out until it connects again.
    fleet_agreed: std::collections::BTreeSet<String>,
    /// #1150: board notices for agents whose machine was not connected when they were made (a merge of an older hub's boards runs at
    /// the first board call after a start, often before every machine is back), at most [`OWED_NOTICES`] per machine, sent when it
    /// connects ([`Hub::take_owed_notices`]). Memory only: a hub that restarts first loses them.
    owed_notices: std::collections::BTreeMap<String, Vec<crate::board::Notice>>,
}

/// The most board notices kept for one machine that is not connected (see `Inner::owed_notices`).
const OWED_NOTICES: usize = 1024;

/// Whether `agent` (`<node>/<agent>`) runs in `project`, as the board's `sync` keeps an agent registered.
fn runs_in(view: &View, agent: &str, project: &str) -> bool {
    view.agents
        .get(agent)
        .is_some_and(|a| a.running && a.project_id == project)
}

pub struct Hub {
    /// The hub's own numbers (`GET /api/metrics`, when `--metrics` turned it on).
    metrics: crate::metrics::Metrics,
    metrics_on: std::sync::atomic::AtomicBool,
    boards: crate::board::Boards,
    /// ADR-0095: each person's recent task dispatches, across all their windows.
    dispatches: crate::dispatch::DispatchRates,
    /// ADR-0113 step 4 and ADR-0114: each person's writes of their settings value and of their plugin grants.
    settings_writes: crate::dispatch::WriteRates,
    grant_writes: crate::dispatch::WriteRates,
    /// ADR-0065: pairing sessions, in memory only.
    pairing: crate::pairing_net::Shared,
    prices: std::sync::RwLock<(std::sync::Arc<crate::usage::Catalog>, u64)>,
    store: Arc<SqliteStore>,
    /// At most one node event job is scheduled on the blocking pool per hub.
    pub(crate) event_writer: Arc<tokio::sync::Semaphore>,
    /// #996: whether a task is retrying the presence facts the store refused (at most one per hub).
    pub(crate) recording_later: std::sync::atomic::AtomicBool,
    router: std::sync::Arc<crate::router::Router>,
    /// ADR-0108: the forgets owed to each node, kept in the store.
    forgets: Arc<crate::forgets::Forgets>,
    inner: Mutex<Inner>,
}

fn decode(e: &crate::store::StoredEvent) -> Option<Delivered> {
    Event::decode(e.payload.as_slice())
        .ok()
        .map(|event| Delivered { pos: e.pos, event })
}

/// ADR-0057: the audit row an event about agents starting agents makes, as `(who, action, node, outcome, at_ms)`. `who` is the asking
/// agent as `<node>/<agent>`. A start is recorded once (an `AgentStarted` said again for an agent the hub knows is not a new start).
fn spawn_audit(
    view: &crate::projection::View,
    event: &Event,
) -> Option<(String, String, String, String, u64)> {
    let node = event.producer_id.clone();
    let q = |id: &str| format!("{node}/{id}");
    let row = |who: String, action: &str, outcome: String| {
        Some((who, action.to_string(), node.clone(), outcome, event.at_ms))
    };
    match event.body.as_ref()? {
        Body::ApprovalRequested(a) if a.action == "agent.start" => {
            row(q(&a.agent_id), "agent.start.asked", a.approval_id.clone())
        }
        Body::ApprovalResolved(r) => {
            let asked = view.approvals.get(&q(&r.approval_id))?;
            if asked.action != "agent.start" {
                return None;
            }
            let outcome = match (r.approved, r.signer_device_id.is_empty()) {
                (true, _) => format!(
                    "approved:{}:grant={}:nested={}",
                    r.signer_device_id, r.helper_grant, r.helpers_may_start
                ),
                (false, false) => format!("denied:{}", r.signer_device_id),
                (false, true) => "withdrawn".to_string(),
            };
            // The view keeps the asking agent by its hub-wide key already.
            row(asked.agent_id.clone(), "agent.start.decided", outcome)
        }
        Body::AgentStartRefused(r) => row(q(&r.agent_id), "agent.start", r.code.clone()),
        Body::AgentStarted(s)
            if !s.parent_agent_id.is_empty() && !view.agents.contains_key(&q(&s.agent_id)) =>
        {
            row(
                q(&s.parent_agent_id),
                "agent.start",
                format!("ok:{}", q(&s.agent_id)),
            )
        }
        // ADR-0059: every start with an option that skips permissions is audited with its options, however it was started (through
        // the hub or at the machine). The node never gives a helper one.
        Body::AgentStarted(s)
            if s.skips_permissions && !view.agents.contains_key(&q(&s.agent_id)) =>
        {
            row(
                q(&s.agent_id),
                "agent.launch_options",
                format!("skips_permissions:{}", s.launch_options.join(",")),
            )
        }
        _ => None,
    }
}

impl Hub {
    /// Opens the hub on a store, rebuilding the view from the log. See [`Hub::open_at`].
    pub fn open(store: SqliteStore) -> Result<Self, StoreError> {
        Self::open_at(store, crate::server::now_ms())
    }

    /// Opens the hub on a store, rebuilding the view from the log, at `now_ms` (hub clock).
    ///
    /// ADR-0070: presence is the hub's fact about live connections, and a hub that has just opened has none. A node the rebuilt
    /// view shows online (its last presence fact says so: the hub stopped without recording the end of its connection, or the
    /// fact is an old `NodeOnline` from before ADR-0070) is recorded offline here, before any connection is accepted: `NodeOffline`
    /// for clients from before ADR-0070, then `NodeLink { up: false, reason: closed }` for the node's latest connection, with
    /// `last_heard_ms` the last time the log heard of the node (not now), so "last seen" stays true. Nothing is recorded for a node
    /// already offline, so opening again adds nothing.
    pub fn open_at(store: SqliteStore, now_ms: u64) -> Result<Self, StoreError> {
        let (view, pos) = View::rebuild(&store)?;
        let hub_seq = store.acked(RESERVED_HUB_ID)?;
        let store = Arc::new(store);
        let hub = Self {
            metrics: crate::metrics::Metrics::default(),
            metrics_on: std::sync::atomic::AtomicBool::new(false),
            dispatches: crate::dispatch::DispatchRates::default(),
            settings_writes: crate::dispatch::WriteRates::new(
                crate::dispatch::MAX_SETTINGS_WRITES,
                crate::dispatch::WRITE_WINDOW,
            ),
            grant_writes: crate::dispatch::WriteRates::new(
                crate::dispatch::MAX_GRANT_WRITES,
                crate::dispatch::WRITE_WINDOW,
            ),
            boards: crate::board::Boards::new(),
            pairing: crate::pairing_net::Shared::new(),
            prices: std::sync::RwLock::new((crate::usage::Catalog::bundled(), 0)),
            forgets: Arc::new(crate::forgets::Forgets::new(Arc::clone(&store))),
            store,
            event_writer: Arc::new(tokio::sync::Semaphore::new(1)),
            recording_later: std::sync::atomic::AtomicBool::new(false),
            router: crate::router::Router::new(),
            inner: Mutex::new(Inner {
                view,
                pos,
                hub_seq,
                subs: Vec::new(),
                subs_dropped: 0,
                approval_wait: crate::metrics::Histogram::with_bounds(&crate::metrics::ANSWER_US),
                unrecorded: std::collections::VecDeque::new(),
                left: std::collections::BTreeSet::new(),
                fleet_agreed: std::collections::BTreeSet::new(),
                owed_notices: std::collections::BTreeMap::new(),
            }),
        };
        {
            let mut g = hub.lock();
            let stale: Vec<(String, u64, u64)> = g
                .view
                .nodes
                .iter()
                .filter(|(_, n)| n.online)
                .map(|(id, n)| (id.clone(), n.link_generation, n.last_event_ms))
                .collect();
            for (node, generation, last_heard_ms) in stale {
                hub.emit(
                    &mut g,
                    Body::NodeOffline(NodeOffline {
                        node_id: node.clone(),
                    }),
                    now_ms,
                )?;
                hub.emit(
                    &mut g,
                    Body::NodeLink(NodeLink {
                        node_id: node,
                        up: false,
                        reason: NodeLinkReason::Closed as i32,
                        generation,
                        last_heard_ms,
                    }),
                    now_ms,
                )?;
            }
        }
        Ok(hub)
    }

    /// Replaces the price table (overrides already applied) and says when it was refreshed (ms since the epoch; 0 for the
    /// table bundled with the release).
    pub fn set_prices(&self, catalog: crate::usage::Catalog, at_ms: u64) {
        if let Ok(mut p) = self.prices.write() {
            *p = (std::sync::Arc::new(catalog), at_ms);
        }
    }

    /// Tokens and estimated cost per agent (ADR-0033). An agent whose model has no price has no estimate; an agent that reported its
    /// own cost in US dollars (ADR-0058) has that instead.
    pub fn usage_report(&self) -> agora_protocol::v1::UsageReportResult {
        use crate::usage::{Cost, Usage};
        let (catalog, prices_at_ms) = match self.prices.read() {
            Ok(p) => (std::sync::Arc::clone(&p.0), p.1),
            Err(_) => (crate::usage::Catalog::bundled(), 0),
        };
        let (view, _) = self.view();
        let mut total = 0u64;
        let agents = view
            .agents
            .iter()
            .map(|(id, a)| {
                let usage = Usage {
                    input_tokens: a.input_tokens,
                    output_tokens: a.output_tokens,
                    ..Usage::default()
                };
                // ADR-0058: a cost the agent reported itself in US dollars is what it cost; only without one is it estimated.
                let reported = a
                    .session
                    .cost_micros
                    .filter(|_| a.session.currency == "USD");
                let micro_usd =
                    reported.or_else(|| catalog.cost(&a.model, &usage, None).map(Cost::micro_usd));
                total = total.saturating_add(micro_usd.unwrap_or(0));
                agora_protocol::v1::AgentUsage {
                    agent_id: id.clone(),
                    model: a.model.clone(),
                    input_tokens: a.input_tokens,
                    output_tokens: a.output_tokens,
                    micro_usd,
                }
            })
            .collect();
        agora_protocol::v1::UsageReportResult {
            agents,
            total_micro_usd: total,
            prices_at_ms,
        }
    }

    /// The pairing state (ADR-0065): the switch, the sessions, the limits. Memory only.
    pub fn pairing(&self) -> &crate::pairing_net::Shared {
        &self.pairing
    }

    /// ADR-0095: the per-person dispatch counts (`task.dispatch_limited`).
    pub fn dispatches(&self) -> &crate::dispatch::DispatchRates {
        &self.dispatches
    }

    /// ADR-0113 step 4: each person's writes of their own settings value (`rate.limited`).
    pub fn settings_writes(&self) -> &crate::dispatch::WriteRates {
        &self.settings_writes
    }

    /// ADR-0114: each person's plugin grant changes (`rate.limited`).
    pub fn grant_writes(&self) -> &crate::dispatch::WriteRates {
        &self.grant_writes
    }

    /// Where connected nodes wait for commands (ADR-0029).
    pub fn router(&self) -> &std::sync::Arc<crate::router::Router> {
        &self.router
    }

    pub fn store(&self) -> &SqliteStore {
        &self.store
    }

    /// ADR-0108: the forgets owed to each node.
    pub fn forgets(&self) -> &Arc<crate::forgets::Forgets> {
        &self.forgets
    }

    pub(crate) fn shared_store(&self) -> Arc<SqliteStore> {
        Arc::clone(&self.store)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock cannot leave the view half-applied (apply is the last step), so keep going.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Turns on `GET /api/metrics` (`agora hub run --metrics`). The numbers are counted either way; this only lets them be read.
    pub fn enable_metrics(&self) {
        self.metrics_on
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether `GET /api/metrics` was turned on.
    pub fn metrics_on(&self) -> bool {
        self.metrics_on.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The hub's numbers in the Prometheus text format, or `None` when they were not turned on.
    pub fn metrics_text(&self) -> Option<String> {
        if !self.metrics_on.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        let gauges = {
            let g = self.lock();
            crate::metrics::Gauges {
                nodes_known: g.view.nodes.len(),
                nodes_online: g.view.nodes.values().filter(|n| n.online).count(),
                subscribers: g.subs.len(),
                subscribers_dropped: g.subs_dropped,
                approvals_waiting: g.view.approvals.len(),
                approval_wait: g.approval_wait.snapshot(),
                relays_dropped: self.router.relays_dropped(),
                terminal_bytes: self.router.terminal_bytes(),
                events_stored: g.pos,
                resyncs: 0,
                node_commands_queued: 0,
                node_commands_unanswered: 0,
            }
        };
        // The router's own lock, taken after the hub's is let go.
        let (node_commands_queued, node_commands_unanswered) = self.router.node_load();
        // Read outside the hub's lock: it is a query on the store.
        let resyncs = self.store.resync_count().unwrap_or(0);
        Some(self.metrics.render(&crate::metrics::Gauges {
            resyncs,
            node_commands_queued,
            node_commands_unanswered,
            ..gauges
        }))
    }

    /// Whether any node by this id has ever been seen (online or not).
    pub fn node_known(&self, node: &str) -> bool {
        self.lock().view.nodes.contains_key(node)
    }

    /// Captures a command's shown target and whether the machine is known, under one projection lock.
    pub(crate) fn audit_target<'a>(
        &self,
        req: &'a agora_protocol::v1::CommandRequest,
    ) -> (crate::audit::Target<'a>, bool) {
        let g = self.lock();
        let target =
            crate::audit::Target::for_command(&req.target_node, req.command.as_ref(), &g.view);
        (target, g.view.nodes.contains_key(&req.target_node))
    }

    /// ADR-0108: the project of `node`'s agent `agent` (its own id), for the audit row of a forget passed on to it; empty when not known.
    pub(crate) fn agent_project(&self, node: &str, agent: &str) -> String {
        self.lock()
            .view
            .agents
            .get(&crate::projection::qualified(node, agent))
            .map(|a| a.project_id.clone())
            .unwrap_or_default()
    }

    /// The current view and the log position it reflects.
    pub fn view(&self) -> (View, u64) {
        let g = self.lock();
        (g.view.clone(), g.pos)
    }

    /// Passes one payload from a node's session through the store, the view and the subscribers.
    pub fn ingest(
        &self,
        session: &mut NodeSession<impl Deref<Target = SqliteStore>>,
        payload: &[u8],
    ) -> Result<Ingest, SessionError> {
        let asked = std::time::Instant::now();
        let mut g = self.lock();
        let held = std::time::Instant::now();
        self.metrics.lock_wait.observe(held - asked);
        let outcome = self.ingest_locked(&mut g, session, payload);
        self.metrics.ingest.observe(held.elapsed());
        outcome
    }

    fn ingest_locked(
        &self,
        g: &mut MutexGuard<'_, Inner>,
        session: &mut NodeSession<impl Deref<Target = SqliteStore>>,
        payload: &[u8],
    ) -> Result<Ingest, SessionError> {
        let (outcome, stored) = session.event_stored(payload)?;
        if let Ingest::Applied { pos, .. } | Ingest::Resynced { pos, .. } = outcome {
            self.metrics
                .events_ingested
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // The session already decoded it successfully, so this cannot fail; stay defensive anyway.
            if let Ok(event) = Event::decode(stored.as_deref().unwrap_or(payload)) {
                let restarted = match &event.body {
                    Some(Body::NodeOnline(n)) => Some((n.node_id.clone(), event.at_ms)),
                    _ => None,
                };
                // Read before the event is folded in: whether an agent is new, and what a decision was about.
                let audit = spawn_audit(&g.view, &event);
                Self::publish(g, Delivered { pos, event });
                if let Some((who, action, node, outcome, at_ms)) = audit {
                    let _ = crate::audit::Audit::new(self.store())
                        .record(at_ms, &who, &action, &node, &outcome);
                }
                // A node announces itself only when its process starts (a resync re-states it only for a start the hub missed,
                // ADR-0055): the agents it had are gone, so their tasks and leases go back.
                if let Some((node, at_ms)) = restarted {
                    for (project, events) in self.boards.node_restarted(&node, at_ms) {
                        if let Some(bytes) = self.boards.snapshot(&project, at_ms) {
                            let _ = self.store.set_setting(&format!("board.{project}"), &bytes);
                        }
                        for body in events {
                            let _ = self.emit(g, body, at_ms);
                        }
                    }
                }
            }
        }
        Ok(outcome)
    }

    /// ADR-0053: the hub stops listing a closed agent. `agent` is the hub-wide key. Refused while it runs; its earlier events stay in
    /// the store, and its tokens and cost still count.
    ///
    /// ADR-0108: the forget is then owed to the agent's node (also for an agent forgotten before, so a forget the node refused while
    /// the agent still ran there can be made again), which removes the saved conversation and the Resume offer once it hears of it
    /// (`server.rs` sends it). Returns the node and the node's own id of the agent.
    pub fn forget_agent(
        &self,
        agent: &str,
        now_ms: u64,
    ) -> Result<(String, String), agora_protocol::Error> {
        use agora_protocol::{Error, ErrorCode};
        let mut g = self.lock();
        let forgotten = match g.view.agents.get(agent) {
            None => return Err(Error::new(ErrorCode::AgentNotFound).with("agent_id", agent)),
            Some(a) if a.running => {
                return Err(Error::new(ErrorCode::AgentRunning).with("agent_id", agent))
            }
            Some(a) => a.forgotten,
        };
        if !forgotten {
            self.emit(
                &mut g,
                Body::AgentForgotten(agora_protocol::v1::AgentForgotten {
                    agent_id: agent.to_string(),
                }),
                now_ms,
            )
            .map_err(|_| Error::new(ErrorCode::Internal))?;
        }
        drop(g);
        let (node, local) = agent.split_once('/').unwrap_or((agent, ""));
        if !local.is_empty() {
            if let Err(e) = self.forgets.add(node, local) {
                // Operational log, not UI text: the hub has forgotten the agent; its node keeps the file until retention removes it.
                eprintln!("agent.forget: {agent}: not owed to its node: {e}");
            }
        }
        Ok((node.to_string(), local.to_string()))
    }

    /// ADR-0073: the hub stops listing an offline machine and what it ran. With `refuse_return`, every certificate enrolled under it
    /// is revoked first, so its next connection is refused until it enrolls again. History stays, and usage still counts.
    pub fn forget_node(
        &self,
        node: &str,
        refuse_return: bool,
        now_ms: u64,
    ) -> Result<(), agora_protocol::Error> {
        use agora_protocol::{Error, ErrorCode};
        let mut g = self.lock();
        // #996: a machine that left while the store refused the record is offline; it is written first so the check sees it.
        self.write_unrecorded(&mut g)
            .map_err(|_| Error::new(ErrorCode::Internal).with("incident_id", "node.forget"))?;
        match g.view.nodes.get(node) {
            None => return Err(Error::new(ErrorCode::NodeUnknown).with("node_id", node)),
            Some(n) if n.online => {
                return Err(Error::new(ErrorCode::NodeOnline).with("node_id", node))
            }
            Some(n) if n.forgotten && !refuse_return => return Ok(()),
            Some(_) => {}
        }
        if refuse_return {
            crate::enroll::Enrollment::new(self.store())
                .revoke(node, now_ms)
                .map_err(|_| Error::new(ErrorCode::Internal).with("incident_id", "node.forget"))?;
        }
        self.emit(
            &mut g,
            Body::NodeForgotten(agora_protocol::v1::NodeForgotten {
                node_id: node.to_string(),
                refused_return: refuse_return,
            }),
            now_ms,
        )
        .map_err(|_| Error::new(ErrorCode::Internal).with("incident_id", "node.forget"))?;
        drop(g);
        // ADR-0108: nothing is owed to a machine the owner forgot (its files go by retention, as before).
        if let Err(e) = self.forgets.drop_node(node) {
            // Operational log, not UI text.
            eprintln!("node.forget: {node}: its pending forgets stay: {e}");
        }
        Ok(())
    }

    /// A node's connection ended without it saying goodbye: record that it is offline.
    pub fn node_disconnected(&self, node: &str, now_ms: u64) -> Result<(), StoreError> {
        let mut g = self.lock();
        self.emit(
            &mut g,
            Body::NodeOffline(NodeOffline {
                node_id: node.to_string(),
            }),
            now_ms,
        )
    }

    /// ADR-0070: a node's connection is open after the handshake. Records `NodeLink { up }` and registers it with the router
    /// (commands reach it from now on) under one lock, so the log's order of presence facts is the order the connections really
    /// took turns in. Returns the registration, the node's commands and this connection's generation. Nothing the node runs is
    /// touched: a reconnect is not a process start.
    ///
    /// #957: when the fact cannot be stored, the connection is not registered and the error is returned, so the caller ends it and
    /// the node dials again (within seconds) and is recorded then. Before, the router took the node and the failure was dropped: the
    /// machine stayed connected and healthy but offline in every view, and nothing ever corrected it.
    pub fn node_link_opened(
        &self,
        node: &str,
        capabilities: Vec<String>,
        now_ms: u64,
    ) -> Result<
        (
            crate::router::Registration,
            tokio::sync::mpsc::UnboundedReceiver<crate::router::ToNode>,
            u64,
        ),
        StoreError,
    > {
        let mut g = self.lock();
        let generation = g
            .view
            .nodes
            .get(node)
            .map_or(0, |n| n.link_generation)
            .saturating_add(1);
        // ADR-0112 Amendment A: a machine is in the hub-wide view only while its latest link agreed on it. A link whose opening the
        // store refuses is not registered, and the agreement of the link before it does not stay: the machine is left out until a
        // link is recorded (review of #1205).
        let agreed = capabilities
            .iter()
            .any(|c| c == agora_protocol::handshake::FLEET_STATUS_V1);
        let opened = self.emit(
            &mut g,
            Body::NodeLink(NodeLink {
                node_id: node.to_string(),
                up: true,
                reason: NodeLinkReason::Opened as i32,
                generation,
                last_heard_ms: now_ms,
            }),
            now_ms,
        );
        if agreed && opened.is_ok() {
            g.fleet_agreed.insert(node.to_string());
        } else {
            g.fleet_agreed.remove(node);
        }
        opened?;
        let (registration, commands) = self.router.register_with_capabilities(node, capabilities);
        Ok((registration, commands, generation))
    }

    /// ADR-0070: a node's connection ended (`silent`: the hub closed it after its deadline). Unregisters it and records
    /// `NodeLink { up: false }` under one lock. When a newer connection of the node had already replaced this one, the fact says
    /// `replaced` and the node stays online; otherwise the hub also records `NodeOffline`, which clients from before ADR-0070
    /// understand. `last_heard_ms` is when the hub last received anything on this connection. Returns whether this was the node's
    /// current connection (the node is offline once the facts are stored).
    ///
    /// #996: facts the store refuses are not dropped. They are kept in order and written before any later hub fact (a node that
    /// dials again is recorded down, then up) and by [`Hub::record_unrecorded`], which the server retries until the store works
    /// ([`Hub::has_unrecorded`]). Before, the failure was dropped: the router had let the node go, but every view kept the machine
    /// online until it came back or the hub restarted.
    pub fn node_link_closed(
        &self,
        registration: crate::router::Registration,
        generation: u64,
        silent: bool,
        last_heard_ms: u64,
        now_ms: u64,
    ) -> bool {
        let mut g = self.lock();
        let node = registration.node().to_string();
        let current = registration.end();
        let reason = match (current, silent) {
            (false, _) => NodeLinkReason::Replaced,
            (true, true) => NodeLinkReason::Silent,
            (true, false) => NodeLinkReason::Closed,
        };
        if current {
            g.unrecorded.push_back((
                Body::NodeOffline(NodeOffline {
                    node_id: node.clone(),
                }),
                now_ms,
            ));
        }
        g.unrecorded.push_back((
            Body::NodeLink(NodeLink {
                node_id: node,
                up: false,
                reason: reason as i32,
                generation,
                last_heard_ms,
            }),
            now_ms,
        ));
        // What the store refuses stays in `unrecorded` (see above); the caller checks `has_unrecorded`.
        let _kept = self.write_unrecorded(&mut g);
        current
    }

    /// #996: writes the presence facts the store refused before, oldest first. `Ok` when none is left.
    pub fn record_unrecorded(&self) -> Result<(), StoreError> {
        let mut g = self.lock();
        self.write_unrecorded(&mut g)
    }

    /// #996: whether presence facts the store refused are still waiting to be written.
    pub fn has_unrecorded(&self) -> bool {
        !self.lock().unrecorded.is_empty()
    }

    /// Writes the facts in `unrecorded` in order, stopping at the first the store refuses (it and the ones after it stay).
    ///
    /// A fact the store does not apply (its number is already taken, or is not the next: the hub's count of its own facts no longer
    /// matches the store's) is an error too, never dropped: it stays, the count is read again from the store, and the next
    /// attempt numbers it from there.
    fn write_unrecorded(&self, g: &mut Inner) -> Result<(), StoreError> {
        while let Some((body, at_ms)) = g.unrecorded.front().cloned() {
            if !self.write(g, body, at_ms)? {
                let seq = g.hub_seq + 1;
                g.hub_seq = self.store.acked(RESERVED_HUB_ID)?;
                return Err(StoreError(format!(
                    "the hub's fact {seq} was not applied; the store holds {}",
                    g.hub_seq
                )));
            }
            g.unrecorded.pop_front();
        }
        Ok(())
    }

    /// Records an event the hub itself produced (as the reserved hub producer). Facts the store refused before are written first
    /// (#996), so the log keeps the order things happened in: a node's late `NodeOffline` can never land after its next `up`.
    fn emit(&self, g: &mut Inner, body: Body, now_ms: u64) -> Result<(), StoreError> {
        self.write_unrecorded(g)?;
        self.write(g, body, now_ms).map(|_| ())
    }

    /// Appends one hub fact and publishes it. Returns whether the store applied it (not a duplicate or a gap).
    fn write(&self, g: &mut Inner, body: Body, now_ms: u64) -> Result<bool, StoreError> {
        let seq = g.hub_seq + 1;
        let event = Event {
            producer_id: RESERVED_HUB_ID.to_string(),
            seq,
            at_ms: now_ms,
            body: Some(body),
        };
        if let Append::Applied { pos } =
            self.store
                .append(RESERVED_HUB_ID, seq, now_ms, &event.encode_to_vec())?
        {
            g.hub_seq = seq;
            Self::publish(g, Delivered { pos, event });
            return Ok(true);
        }
        Ok(false)
    }

    /// ADR-0039 amendment B: the `(node, project)` pairs that must hand over their local work now (see `Boards::membership`).
    ///
    /// #1036: a node the log took offline since the last look is no longer a remembered member, even if it is online again now
    /// (its end and its next `up` written in one step), so it is asked as after any other return.
    pub fn board_membership(&self, now_ms: u64) -> Vec<(String, String)> {
        let mut g = self.lock();
        let left = std::mem::take(&mut g.left);
        self.boards.forget_members(&left);
        self.boards.membership(&g.view, now_ms)
    }

    /// #1150: the board notices kept for `node` while it was not connected (see `Inner::owed_notices`), for the connection that
    /// just opened to send; each is given once.
    pub fn take_owed_notices(&self, node: &str) -> Vec<crate::board::Notice> {
        self.lock().owed_notices.remove(node).unwrap_or_default()
    }

    /// A node will not answer its `BoardJoin`: board calls stop waiting for it.
    pub fn board_join_settled(&self, node: &str, project: &str) {
        self.boards.join_settled(node, project);
    }

    /// Puts what `node` handed over for `project` on the board, publishes the board's events and returns the notices for agents.
    pub fn board_joined(
        &self,
        node: &str,
        project: &str,
        work: &agora_protocol::v1::BoardJoinResult,
        now_ms: u64,
    ) -> Vec<crate::board::Notice> {
        let mut g = self.lock();
        let (key, mut notices) = self.restore_board(&mut g, project, now_ms);
        let (events, joined) = self.boards.joined(&g.view, node, project, work, now_ms);
        notices.extend(joined);
        if !events.is_empty() {
            if let Some(bytes) = self.boards.snapshot(project, now_ms) {
                let _ = self.store.set_setting(&key, &bytes);
            }
        }
        for body in events {
            let _ = self.emit(&mut g, body, now_ms);
        }
        notices
    }

    /// A handoff could not be delivered after the board accepted it: give the task back to the giver.
    pub fn board_revert(&self, h: &crate::board::Handoff, now_ms: u64) {
        let mut g = self.lock();
        let events = self.boards.revert_handoff(h);
        if !events.is_empty() {
            if let Some(bytes) = self.boards.snapshot(&h.project_id, now_ms) {
                let _ = self
                    .store
                    .set_setting(&format!("board.{}", h.project_id), &bytes);
            }
        }
        for body in events {
            let _ = self.emit(&mut g, body, now_ms);
        }
    }

    /// A board tool call from `node` (ADR-0039). The caller's identity is the connection's, never the call's. Events the
    /// board produced are published here; a handoff to pass on to another node is returned for the caller to forward.
    pub fn board_call(
        &self,
        node: &str,
        call: &agora_protocol::v1::BoardCall,
        now_ms: u64,
    ) -> crate::board::Answer {
        let refused = || crate::board::Answer {
            verdict: crate::board::Verdict::Invalid(
                agora_protocol::Error::new(agora_protocol::ErrorCode::Internal)
                    .with("incident_id", "board.store"),
            ),
            events: Vec::new(),
            handoff: None,
            fanout: None,
            notices: Vec::new(),
        };
        let mut g = self.lock();
        // #996: a machine that left while the store refused the record is offline before the board decides anything (a handoff
        // to it is refused), and the board's events come after that record in the log.
        if self.write_unrecorded(&mut g).is_err() {
            return refused();
        }
        // ADR-0112: the hub-wide view for agents is read only and belongs to no project's board.
        if call.tool == "fleet_status" {
            return self.fleet_call(&g.view, &g.fleet_agreed, call);
        }
        // ADR-0115: an older node names a repository on a case-insensitive host in its own case; the board is the folded project's.
        let project = agora_core::fold_project_id(&call.project_id);
        let project = project.as_ref();
        let (key, notices) = self.restore_board(&mut g, project, now_ms);
        // A change is answered as done only once it is stored, and the board with its audit events is stored in one transaction:
        // all of it or none (#517 review: a partly written change published one grant of a refused lease batch, and a failed undo
        // brought a refused claim back after a restart). Until then the board as it was is kept aside, to be put back.
        let before =
            crate::board::changes_board(&call.tool).then(|| self.boards.checkpoint(project));
        let mut a = self.boards.call(
            &g.view,
            node,
            &call.from_agent,
            project,
            &call.tool,
            &call.arguments_json,
            now_ms,
        );
        a.notices.extend(notices);
        if a.events.is_empty() {
            return a;
        }
        let events = std::mem::take(&mut a.events);
        // Only once stored does anyone hear of it: the view, the windows and the nodes.
        match self.store_board(&mut g, project, &key, events, &[], now_ms) {
            Ok(()) => a,
            Err(_) => {
                if let Some(board) = before {
                    self.boards.restore(project, board);
                }
                let mut r = refused();
                r.notices = a.notices;
                r
            }
        }
    }

    /// ADR-0112: `fleet_status`, answered for a solo hub, or a team hub whose owner turned it on; refused `board.not_allowed`
    /// otherwise. It names no project and takes no arguments.
    fn fleet_call(
        &self,
        view: &crate::projection::View,
        agreed: &std::collections::BTreeSet<String>,
        call: &agora_protocol::v1::BoardCall,
    ) -> crate::board::Answer {
        let invalid = |field: &str| crate::board::Answer {
            verdict: crate::board::Verdict::Invalid(
                agora_protocol::Error::new(agora_protocol::ErrorCode::ProtocolInvalidMessage)
                    .with("field", field),
            ),
            events: Vec::new(),
            handoff: None,
            fanout: None,
            notices: Vec::new(),
        };
        if !call.project_id.is_empty() {
            return invalid("project_id");
        }
        if !serde_json::from_str::<serde_json::Value>(&call.arguments_json)
            .is_ok_and(|v| v.is_object())
        {
            return invalid("arguments_json");
        }
        if !crate::users::fleet_allowed(&self.store) {
            return crate::board::fleet_refused();
        }
        crate::board::fleet_answer(view, agreed)
    }

    /// Restores an uninitialized board and returns its storage key, with the notices the restore owes agents. Callers hold the hub
    /// lock so calls and joins cannot reload over one another's newer in-memory work.
    fn restore_board(
        &self,
        g: &mut Inner,
        project: &str,
        now_ms: u64,
    ) -> (String, Vec<crate::board::Notice>) {
        let key = format!("board.{project}");
        if self.boards.has(project) {
            return (key, Vec::new());
        }
        let mut parts = Vec::new();
        if let Ok(Some(bytes)) = self.store.setting(&key) {
            parts.push(bytes);
        }
        let own = parts.first().cloned();
        // ADR-0115: boards an older hub stored under the project's spellings in another case (one per spelling it saw). Each is
        // merged into the folded project's board once (`board-merged.<spelling>` remembers it, so a restart does not add it twice)
        // and the result is stored under the folded id; the old records are left for a downgrade.
        let spellings: std::collections::BTreeSet<String> = g
            .view
            .spellings_of(project)
            .filter(|s| *s != project)
            .map(str::to_string)
            .collect();
        let mut merged = Vec::new();
        for spelling in spellings {
            let marker = format!("board-merged.{spelling}");
            if !matches!(self.store.setting(&marker), Ok(None)) {
                continue;
            }
            if let Ok(Some(bytes)) = self.store.setting(&format!("board.{spelling}")) {
                parts.push(bytes);
                merged.push(marker);
            }
        }
        match (parts.len(), merged.is_empty()) {
            (0, _) => {}
            (1, true) => self.boards.load(project, &parts[0], now_ms),
            _ => {
                let m = self
                    .boards
                    .merge_snapshots(&parts, now_ms, |a| runs_in(&g.view, a, project));
                let none = self.boards.checkpoint(project);
                self.boards.load(project, &m.bytes, now_ms);
                // #1150: the window learns what the merge changed (a task with a new id, and the one that kept the id it had), and
                // the merged board is stored with its markers and those events in one transaction. A board stored without its
                // markers would be merged again at the next start and its tasks added twice; so if the store refuses, the folded
                // board is used alone, as it was stored, and the merge is tried again at the next start.
                let changes = self.boards.task_changes(project, &m.restated);
                let markers: Vec<(String, Vec<u8>)> = merged
                    .into_iter()
                    .map(|marker| (marker, project.as_bytes().to_vec()))
                    .collect();
                let stored = self.write_unrecorded(g).is_ok()
                    && self
                        .store_board(g, project, &key, changes, &markers, now_ms)
                        .is_ok();
                if !stored {
                    self.boards.restore(project, none);
                    if let Some(bytes) = own {
                        self.boards.load(project, &bytes, now_ms);
                    }
                    return (key, Vec::new());
                }
                // As after a join, only an agent running in the project is told (a node that started again lost its agents). One
                // whose machine is not connected now (after a hub start, before it is back) is told when it connects: the markers
                // are stored, so this merge will not run again to tell it.
                let mut notices = Vec::new();
                for n in m.notices {
                    if !runs_in(&g.view, &format!("{}/{}", n.to_node, n.to_agent), project) {
                        continue;
                    }
                    if self.router.is_connected(&n.to_node) {
                        notices.push(n);
                    } else {
                        let owed = g.owed_notices.entry(n.to_node.clone()).or_default();
                        if owed.len() < OWED_NOTICES {
                            owed.push(n);
                        }
                    }
                }
                return (key, notices);
            }
        }
        (key, Vec::new())
    }

    /// Stores `project`'s board under `key` with the hub's events `bodies` and the further `settings`, in one transaction, and
    /// publishes the events once stored.
    fn store_board(
        &self,
        g: &mut Inner,
        project: &str,
        key: &str,
        bodies: Vec<Body>,
        settings: &[(String, Vec<u8>)],
        now_ms: u64,
    ) -> Result<(), StoreError> {
        let snapshot = self
            .boards
            .snapshot(project, now_ms)
            .ok_or_else(|| StoreError(format!("no board for {project}")))?;
        let first = g.hub_seq + 1;
        let events: Vec<Event> = bodies
            .into_iter()
            .zip(first..)
            .map(|(body, seq)| Event {
                producer_id: RESERVED_HUB_ID.to_string(),
                seq,
                at_ms: now_ms,
                body: Some(body),
            })
            .collect();
        let payloads: Vec<(u64, Vec<u8>)> =
            events.iter().map(|e| (now_ms, e.encode_to_vec())).collect();
        let all: Vec<(&str, &[u8])> = std::iter::once((key, snapshot.as_slice()))
            .chain(settings.iter().map(|(k, v)| (k.as_str(), v.as_slice())))
            .collect();
        let positions =
            self.store
                .append_batch_with_settings(RESERVED_HUB_ID, first, &payloads, &all)?;
        g.hub_seq += events.len() as u64;
        for (pos, event) in positions.into_iter().zip(events) {
            Self::publish(g, Delivered { pos, event });
        }
        Ok(())
    }

    /// ADR-0115: `req` as `node` should get it: a project it names in the spelling that node last reported for it (an older node
    /// spells a repository on a case-insensitive host in its own case and knows it only so). Commands that name no project, and every
    /// command to a node that spells it folded, are unchanged. A launch keeps its project as signed (the node answered it itself).
    pub fn spelled_for(
        &self,
        node: &str,
        mut req: agora_protocol::v1::CommandRequest,
    ) -> agora_protocol::v1::CommandRequest {
        use agora_protocol::v1::command_request::Command as C;
        let project = match req.command.as_mut() {
            Some(C::AgentStart(c)) => &mut c.project_id,
            Some(C::TaskUpdate(c)) => &mut c.project_id,
            Some(C::TaskDispatch(c)) => &mut c.project_id,
            Some(C::BoardJoin(c)) => &mut c.project_id,
            Some(C::DeliverTaskHandoff(c)) => &mut c.project_id,
            _ => return req,
        };
        let g = self.lock();
        let spelled = g.view.spelled_for(node, project);
        if spelled != project.as_str() {
            *project = spelled.to_string();
        }
        req
    }

    fn publish(g: &mut Inner, d: Delivered) {
        // An answer to a waiting approval: how long the agent waited, both times on the asking machine's clock.
        if let Some(Body::ApprovalResolved(r)) = &d.event.body {
            let key = crate::projection::qualified(&d.event.producer_id, &r.approval_id);
            if let Some(a) = g.view.approvals.get(&key) {
                let waited = d.event.at_ms.saturating_sub(a.requested_at_ms);
                g.approval_wait
                    .observe(std::time::Duration::from_millis(waited));
            }
        }
        let back = g.view.brings_back(&d.event);
        // #1036: whether this fact takes a node offline, for board membership's next look.
        let presence = presence_of(&d.event).map(|node| {
            let online = g.view.nodes.get(node).is_some_and(|n| n.online);
            (node.to_string(), online)
        });
        g.view.apply_at(d.pos, &d.event);
        if let Some((node, true)) = presence {
            if !g.view.nodes.get(&node).is_some_and(|n| n.online) {
                g.left.insert(node);
            }
        }
        g.pos = d.pos;
        // ADR-0073: a forgotten machine came back. A client that joined while it was hidden holds none of its folders or agents, and
        // no event will say them again; every client is let go, reconnects from its position and, being before this one, gets a
        // snapshot (see `subscribe_within`).
        if back {
            g.view.remembered_at = d.pos;
            g.subs.clear();
            return;
        }
        // `try_send`: a full queue means the subscriber is too slow; drop it rather than stall every other node.
        let before = g.subs.len();
        g.subs.retain(|s| match s.try_send(d.clone()) {
            Ok(()) => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => false,
        });
        g.subs_dropped += (before - g.subs.len()) as u64;
    }

    /// The stored events after `after`, or `None` when they come to more than `max_bytes`: then a snapshot is cheaper and bounded.
    /// The size is asked of the store first, so a replay over budget copies no payload at all (#519 review: a page of 1 000 large
    /// events was read into memory before the check).
    fn replay(&self, after: u64, max_bytes: usize) -> Result<Option<Vec<Delivered>>, StoreError> {
        if self.store.payload_bytes_after(after)? > u64::try_from(max_bytes).unwrap_or(u64::MAX) {
            return Ok(None);
        }
        let mut backlog = Vec::new();
        let (mut at, mut bytes) = (after, 0usize);
        loop {
            let page = self.store.events_after(at, PAGE)?;
            let Some(last) = page.last() else { break };
            at = last.pos;
            bytes += page.iter().map(|e| e.payload.len()).sum::<usize>();
            if bytes > max_bytes {
                return Ok(None);
            }
            backlog.extend(page.iter().filter_map(decode));
        }
        Ok(Some(backlog))
    }

    /// Subscribes from `after` (the last position the subscriber saw), or from the start with `None`. If `after`
    /// is ahead of the log (the hub was restored from an older backup), or further behind than [`MAX_REPLAY_EVENTS`] or
    /// [`MAX_REPLAY_BYTES`] of replay, the subscriber gets a snapshot instead.
    pub fn subscribe(&self, after: Option<u64>) -> Result<Subscription, StoreError> {
        self.subscribe_within(after, MAX_REPLAY_EVENTS, MAX_REPLAY_BYTES)
    }

    fn subscribe_within(
        &self,
        after: Option<u64>,
        max_events: u64,
        max_bytes: usize,
    ) -> Result<Subscription, StoreError> {
        let mut g = self.lock();
        let (tx, rx) = sync_channel(SUBSCRIBER_BUFFER);
        // Positions only grow (one per stored event), so `pos - n` bounds the number of events to replay before any is read.
        let backlog = match after {
            Some(n)
                if n <= g.pos
                    && g.pos - n <= max_events
                    && n >= g.view.remembered_at
                    && (g.view.distrusted_until == 0 || n > g.view.distrusted_until) =>
            {
                self.replay(n, max_bytes)?
            }
            _ => None,
        };
        let sub = match backlog {
            Some(backlog) => Subscription {
                snapshot: None,
                backlog,
                live: rx,
            },
            None => Subscription {
                snapshot: Some((g.view.clone(), g.pos)),
                backlog: Vec::new(),
                live: rx,
            },
        };
        g.subs.push(tx);
        Ok(sub)
    }

    pub fn subscriber_count(&self) -> usize {
        self.lock().subs.len()
    }
}

/// The node a presence fact can take offline (the view decides whether it does): the one a hub's `NodeOffline` or `NodeLink` names,
/// or the sender of a node's own `NodeOffline` (see `View::apply`).
fn presence_of(event: &Event) -> Option<&str> {
    match &event.body {
        Some(Body::NodeOffline(e)) if event.producer_id == RESERVED_HUB_ID => Some(&e.node_id),
        Some(Body::NodeOffline(_)) => Some(&event.producer_id),
        Some(Body::NodeLink(e)) if !e.up => Some(&e.node_id),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_protocol::handshake::Support;
    use agora_protocol::v1::{AgentStarted, Hello, NodeOnline, Resume, Role};

    #[test]
    fn agent_starts_are_audited_once_with_the_asking_agent() {
        use agora_protocol::v1::{AgentStartRefused, ApprovalRequested, ApprovalResolved};
        let mut view = View::default();
        let at = |body: Body| Event {
            producer_id: "n1".into(),
            seq: 1,
            at_ms: 5,
            body: Some(body),
        };
        let asked = at(Body::ApprovalRequested(ApprovalRequested {
            approval_id: "ap-1".into(),
            agent_id: "a".into(),
            action: "agent.start".into(),
            detail: "{}".into(),
            expires_at_ms: 9,
            ..Default::default()
        }));
        let row = spawn_audit(&view, &asked).unwrap();
        assert_eq!(
            (row.0.as_str(), row.1.as_str(), row.3.as_str()),
            ("n1/a", "agent.start.asked", "ap-1")
        );
        view.apply(&asked);
        let decided = at(Body::ApprovalResolved(ApprovalResolved {
            approval_id: "ap-1".into(),
            approved: true,
            signer_device_id: "phone".into(),
            helper_grant: 3,
            helpers_may_start: false,
            ..Default::default()
        }));
        let row = spawn_audit(&view, &decided).unwrap();
        assert_eq!(
            (row.0.as_str(), row.3.as_str()),
            ("n1/a", "approved:phone:grant=3:nested=false")
        );
        let refused = at(Body::AgentStartRefused(AgentStartRefused {
            agent_id: "a".into(),
            kind: "codex".into(),
            folder: "/w".into(),
            code: "mesh.helper_limit".into(),
        }));
        assert_eq!(spawn_audit(&view, &refused).unwrap().3, "mesh.helper_limit");
        let started = at(Body::AgentStarted(AgentStarted {
            run_id: Vec::new(),
            agent_id: "b".into(),
            project_id: "p".into(),
            node_id: "n1".into(),
            worktree_id: String::new(),
            kind: "codex".into(),
            level: 1,
            parent_agent_id: "a".into(),
            same_process: false,
            launch_options: Vec::new(),
            skips_permissions: false,
        }));
        let row = spawn_audit(&view, &started).unwrap();
        assert_eq!((row.0.as_str(), row.3.as_str()), ("n1/a", "ok:n1/b"));
        view.apply(&started);
        // Said again (a restart or a resync): not a new start.
        assert!(spawn_audit(&view, &started).is_none());
    }

    #[test]
    fn a_start_that_does_not_ask_is_audited_once_with_its_options() {
        let mut view = crate::projection::View::default();
        let at = |body| Event {
            producer_id: "n1".into(),
            seq: 1,
            at_ms: 7,
            body: Some(body),
        };
        let started = |skips, options: &[&str]| {
            at(Body::AgentStarted(AgentStarted {
                run_id: Vec::new(),
                agent_id: "c".into(),
                project_id: "p".into(),
                node_id: "n1".into(),
                worktree_id: String::new(),
                kind: "codex".into(),
                level: 1,
                parent_agent_id: String::new(),
                same_process: false,
                launch_options: options.iter().map(|o| (*o).to_string()).collect(),
                skips_permissions: skips,
            }))
        };
        assert!(spawn_audit(&view, &started(false, &[])).is_none());
        assert!(spawn_audit(&view, &started(false, &["resume-last"])).is_none());
        let skip = started(true, &["skip-permissions"]);
        let row = spawn_audit(&view, &skip).unwrap();
        assert_eq!(
            (row.0.as_str(), row.1.as_str(), row.3.as_str()),
            (
                "n1/c",
                "agent.launch_options",
                "skips_permissions:skip-permissions"
            )
        );
        view.apply(&skip);
        assert!(spawn_audit(&view, &skip).is_none());
    }

    fn support() -> Support {
        Support {
            generations: vec![1],
            capabilities: vec![],
            required: vec![],
        }
    }

    fn hello() -> Hello {
        Hello {
            role: Role::Node as i32,
            protocol_generations: vec![1],
            capabilities: vec![],
            build: "t".into(),
            locale: None,
        }
    }

    fn open_session<'a>(
        hub: &'a Hub,
        sup: &'a Support,
        node: &str,
    ) -> NodeSession<&'a SqliteStore> {
        let mut s = NodeSession::new(hub.store(), node, sup);
        s.hello(&hello(), 0).unwrap();
        s.resume(&Resume::default()).unwrap();
        s
    }

    fn online(node: &str, seq: u64) -> Vec<u8> {
        Event {
            producer_id: node.into(),
            seq,
            at_ms: seq,
            body: Some(Body::NodeOnline(NodeOnline {
                node_id: node.into(),
                hostname: "h".into(),
                os: "linux".into(),
                build: "b".into(),
                binary: String::new(),
            })),
        }
        .encode_to_vec()
    }

    fn agent(node: &str, seq: u64, id: &str) -> Vec<u8> {
        Event {
            producer_id: node.into(),
            seq,
            at_ms: seq,
            body: Some(Body::AgentStarted(AgentStarted {
                run_id: Vec::new(),
                agent_id: id.into(),
                project_id: "p".into(),
                node_id: node.into(),
                worktree_id: "w".into(),
                kind: "shell".into(),
                level: 2,
                parent_agent_id: String::new(),
                same_process: false,
                launch_options: Vec::new(),
                skips_permissions: false,
            })),
        }
        .encode_to_vec()
    }

    fn hub() -> Hub {
        Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()
    }

    #[test]
    fn ingest_updates_the_view_and_the_log_together() {
        let (hub, sup) = (hub(), support());
        let mut s = open_session(&hub, &sup, "n1");
        hub.ingest(&mut s, &online("n1", 1)).unwrap();
        hub.ingest(&mut s, &agent("n1", 2, "a")).unwrap();
        let (view, pos) = hub.view();
        assert_eq!(pos, 2);
        assert!(view.nodes["n1"].online);
        assert!(view.agents.contains_key("n1/a"));
        // A duplicate changes nothing and is not published.
        let sub = hub.subscribe(Some(2)).unwrap();
        assert_eq!(
            hub.ingest(&mut s, &agent("n1", 2, "a")).unwrap(),
            Ingest::Duplicate { acked: 2 }
        );
        assert!(sub.live.try_recv().is_err());
    }

    /// ADR-0054: the event a node's resync lands on goes into the view and to subscribers like any applied one.
    #[test]
    fn a_resynced_event_is_published_and_folded_into_the_view() {
        let (hub, sup) = (hub(), support());
        let mut s = open_session(&hub, &sup, "n1");
        hub.ingest(&mut s, &online("n1", 1)).unwrap();
        let sub = hub.subscribe(Some(1)).unwrap();
        s.snapshot_required(9).unwrap();
        assert_eq!(
            hub.ingest(&mut s, &agent("n1", 7, "a")).unwrap(),
            Ingest::Resynced {
                acked: 7,
                pos: 2,
                from_seq: 2,
                to_seq: 6
            }
        );
        assert!(hub.view().0.agents.contains_key("n1/a"));
        assert_eq!(sub.live.try_recv().map(|d| d.pos).ok(), Some(2));
    }

    #[test]
    fn a_closed_agent_can_be_forgotten_but_a_running_one_cannot() {
        use agora_protocol::{v1::AgentExited, ErrorCode};
        let (hub, sup) = (hub(), support());
        let mut s = open_session(&hub, &sup, "n1");
        hub.ingest(&mut s, &online("n1", 1)).unwrap();
        hub.ingest(&mut s, &agent("n1", 2, "a")).unwrap();
        // Running: refused, and nothing was recorded.
        let e = hub.forget_agent("n1/a", 5).unwrap_err();
        assert_eq!(e.code, ErrorCode::AgentRunning);
        assert_eq!(hub.view().1, 2);
        assert_eq!(
            hub.forget_agent("n1/nobody", 5).unwrap_err().code,
            ErrorCode::AgentNotFound
        );
        let exited = Event {
            producer_id: "n1".into(),
            seq: 3,
            at_ms: 3,
            body: Some(Body::AgentExited(AgentExited {
                agent_id: "a".into(),
                exit_code: 0,
            })),
        }
        .encode_to_vec();
        hub.ingest(&mut s, &exited).unwrap();
        let sub = hub.subscribe(Some(3)).unwrap();
        hub.forget_agent("n1/a", 6).unwrap();
        // Clients hear of it as an event; the event is in the log, the agent's history is not rewritten, and asking again is harmless.
        let heard = sub.live.try_recv().unwrap();
        assert!(matches!(heard.event.body, Some(Body::AgentForgotten(_))));
        assert_eq!(hub.view().1, 4);
        hub.forget_agent("n1/a", 7).unwrap();
        assert_eq!(hub.view().1, 4);
        let (view, _) = hub.view();
        assert!(view.agents["n1/a"].forgotten);
        assert!(!view.agents["n1/a"].running);
    }

    /// ADR-0108: a forget is owed to the agent's node in the store, so a hub restart keeps it; a second forget is owed once; forgetting
    /// the machine drops what it was owed.
    #[test]
    fn a_forget_is_owed_to_the_agents_node_across_a_restart_until_the_machine_is_forgotten() {
        use agora_protocol::v1::AgentExited;
        let path =
            std::env::temp_dir().join(format!("agora-hub-forgets-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let sup = support();
        {
            let hub = Hub::open(SqliteStore::open(&path).unwrap()).unwrap();
            let mut s = open_session(&hub, &sup, "n1");
            hub.ingest(&mut s, &online("n1", 1)).unwrap();
            hub.ingest(&mut s, &agent("n1", 2, "agent-3")).unwrap();
            assert!(hub.forget_agent("n1/agent-3", 3).is_err(), "it runs");
            assert!(
                hub.forgets().pending("n1").is_empty(),
                "a refused forget is owed to nobody"
            );
            let exited = Event {
                producer_id: "n1".into(),
                seq: 3,
                at_ms: 3,
                body: Some(Body::AgentExited(AgentExited {
                    agent_id: "agent-3".into(),
                    exit_code: 0,
                })),
            };
            hub.ingest(&mut s, &exited.encode_to_vec()).unwrap();
            assert_eq!(
                hub.forget_agent("n1/agent-3", 4).unwrap(),
                ("n1".to_string(), "agent-3".to_string())
            );
            hub.forget_agent("n1/agent-3", 5).unwrap();
            assert_eq!(hub.forgets().pending("n1"), vec!["agent-3"]);
        }
        // A new hub process on the same store: still owed.
        let hub = Hub::open(SqliteStore::open(&path).unwrap()).unwrap();
        assert_eq!(hub.forgets().pending("n1"), vec!["agent-3"]);
        hub.forgets().answered("n1", "agent-3").unwrap();
        assert!(hub.forgets().pending("n1").is_empty());
        // The machine is forgotten: what it was owed goes.
        hub.forget_agent("n1/agent-3", 6).unwrap();
        assert_eq!(hub.forgets().pending("n1"), vec!["agent-3"]);
        hub.forget_node("n1", false, 7).unwrap();
        assert!(hub.forgets().pending("n1").is_empty());
        drop(hub);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_offline_machine_can_be_forgotten_and_comes_back_when_it_connects() {
        use agora_protocol::{v1::AgentExited, ErrorCode};
        let (hub, sup) = (hub(), support());
        let mut s = open_session(&hub, &sup, "n1");
        let (reg, _cmds, generation) = hub.node_link_opened("n1", Vec::new(), 1).unwrap();
        hub.ingest(&mut s, &online("n1", 1)).unwrap();
        hub.ingest(&mut s, &agent("n1", 2, "closed")).unwrap();
        hub.ingest(&mut s, &agent("n1", 3, "running")).unwrap();
        let exited = Event {
            producer_id: "n1".into(),
            seq: 4,
            at_ms: 4,
            body: Some(Body::AgentExited(AgentExited {
                agent_id: "closed".into(),
                exit_code: 0,
            })),
        };
        hub.ingest(&mut s, &exited.encode_to_vec()).unwrap();
        // Connected: refused, nothing recorded. Unknown: refused.
        let before = hub.view().1;
        assert_eq!(
            hub.forget_node("n1", false, 5).unwrap_err().code,
            ErrorCode::NodeOnline
        );
        assert_eq!(
            hub.forget_node("nobody", false, 5).unwrap_err().code,
            ErrorCode::NodeUnknown
        );
        assert_eq!(hub.view().1, before);

        hub.node_link_closed(reg, generation, false, 5, 5);
        let sub = hub.subscribe(Some(hub.view().1)).unwrap();
        hub.forget_node("n1", false, 6).unwrap();
        let heard = sub.live.try_recv().unwrap();
        assert!(matches!(heard.event.body, Some(Body::NodeForgotten(_))));
        // Kept in the hub's records, left out of what clients are shown.
        let (view, _) = hub.view();
        assert!(view.nodes["n1"].forgotten);
        assert!(view.agents["n1/closed"].forgotten && view.agents["n1/running"].forgotten);
        let shown = crate::server::snapshot_of(&view, 0);
        assert!(shown.nodes.is_empty(), "{:?}", shown.nodes);
        assert!(shown.agents.is_empty(), "{:?}", shown.agents);
        // Asking again is harmless.
        let at = hub.view().1;
        hub.forget_node("n1", false, 7).unwrap();
        assert_eq!(hub.view().1, at);

        // It connects again: listed again, with the agent it still runs; the one closed before stays forgotten.
        let _back = hub.node_link_opened("n1", Vec::new(), 8).unwrap();
        let (view, _) = hub.view();
        assert!(!view.nodes["n1"].forgotten);
        let shown = crate::server::snapshot_of(&view, 0);
        assert_eq!(shown.nodes.len(), 1);
        let agents: Vec<&str> = shown.agents.iter().map(|a| a.agent_id.as_str()).collect();
        assert_eq!(agents, ["n1/running"]);
    }

    #[test]
    fn a_client_that_joined_while_a_machine_was_forgotten_gets_its_records_when_it_comes_back() {
        // #535 review (Codex): a client that subscribed while the machine was hidden holds none of its folders or agents, and the
        // machine's return is only a `NodeLink`; it must be given a snapshot, not a replay.
        use agora_protocol::v1::CheckoutDiscovered;
        let (hub, sup) = (hub(), support());
        let mut s = open_session(&hub, &sup, "n1");
        let (reg, _cmds, generation) = hub.node_link_opened("n1", Vec::new(), 1).unwrap();
        hub.ingest(&mut s, &online("n1", 1)).unwrap();
        let checkout = Event {
            producer_id: "n1".into(),
            seq: 2,
            at_ms: 2,
            body: Some(Body::CheckoutDiscovered(CheckoutDiscovered {
                checkout_id: "c".into(),
                node_id: "n1".into(),
                project_id: "p".into(),
                path: "/repo".into(),
            })),
        };
        hub.ingest(&mut s, &checkout.encode_to_vec()).unwrap();
        hub.ingest(&mut s, &agent("n1", 3, "running")).unwrap();
        hub.node_link_closed(reg, generation, false, 4, 4);
        hub.forget_node("n1", false, 5).unwrap();

        // This client joins while the machine is hidden: its snapshot has none of it.
        let joined = hub.subscribe(None).unwrap();
        let (view, at) = joined.snapshot.clone().unwrap();
        let shown = crate::server::snapshot_of(&view, at);
        assert!(shown.nodes.is_empty() && shown.checkouts.is_empty() && shown.agents.is_empty());

        // The machine comes back: every live client is let go...
        let _back = hub.node_link_opened("n1", Vec::new(), 6).unwrap();
        assert!(
            joined.live.try_recv().is_err(),
            "a live client was kept: it would only hear NodeLink"
        );
        // ...and resuming from where it was gives a snapshot with the machine's folders and running agent, not a replay.
        let again = hub.subscribe(Some(at)).unwrap();
        assert!(again.backlog.is_empty());
        let (view, pos) = again.snapshot.expect("a snapshot");
        let shown = crate::server::snapshot_of(&view, pos);
        assert_eq!(shown.nodes.len(), 1);
        assert_eq!(shown.checkouts.len(), 1);
        assert_eq!(
            shown
                .agents
                .iter()
                .map(|a| a.agent_id.as_str())
                .collect::<Vec<_>>(),
            ["n1/running"]
        );
        // A client that resumes from after the return replays as usual.
        let after = hub.subscribe(Some(pos)).unwrap();
        assert!(after.snapshot.is_none());
        // The hub restarting keeps the rule: the rebuilt view knows where the machine came back.
        assert_eq!(View::rebuild(hub.store()).unwrap().0.remembered_at, pos);
    }

    #[test]
    fn forgetting_with_refuse_return_revokes_the_machines_certificates() {
        let (hub, sup) = (hub(), support());
        let ca = crate::ca::Ca::generate("ca").unwrap();
        let (_, csr) = crate::ca::new_device_request().unwrap();
        let mut e = crate::enroll::Enrollment::new(hub.store());
        let code = e.create_code(1).unwrap();
        let issued = e.redeem(&ca, &code, "n1", &csr, 2).unwrap();
        let mut s = open_session(&hub, &sup, "n1");
        hub.ingest(&mut s, &online("n1", 1)).unwrap();
        hub.node_disconnected("n1", 3).unwrap();
        hub.forget_node("n1", true, 4).unwrap();
        assert!(!crate::enroll::Enrollment::new(hub.store())
            .is_active(&issued.serial_hex)
            .unwrap());
        let (view, _) = hub.view();
        assert!(view.nodes["n1"].forgotten);
    }

    #[test]
    fn a_new_subscriber_gets_a_snapshot_then_live_events() {
        let (hub, sup) = (hub(), support());
        let mut s = open_session(&hub, &sup, "n1");
        hub.ingest(&mut s, &online("n1", 1)).unwrap();
        let sub = hub.subscribe(None).unwrap();
        let (view, pos) = sub.snapshot.unwrap();
        assert_eq!((view.nodes.len(), pos), (1, 1));
        assert!(sub.backlog.is_empty());
        hub.ingest(&mut s, &agent("n1", 2, "a")).unwrap();
        let live = sub.live.try_recv().unwrap();
        assert_eq!(live.pos, 2);
    }

    #[test]
    fn a_subscriber_too_far_behind_gets_a_snapshot_not_an_unbounded_replay() {
        let (hub, sup) = (hub(), support());
        let mut s = open_session(&hub, &sup, "n1");
        for i in 1..=6 {
            hub.ingest(&mut s, &agent("n1", i, &format!("a{i}")))
                .unwrap();
        }
        let head = hub.view().1;
        // Within the budget: the backlog, as before.
        let near = hub.subscribe_within(Some(head - 2), 2, usize::MAX).unwrap();
        assert!(near.snapshot.is_none());
        assert_eq!(near.backlog.len(), 2);
        // More events behind than the budget: a snapshot at the head, nothing replayed.
        let far = hub.subscribe_within(Some(head - 3), 2, usize::MAX).unwrap();
        assert!(far.backlog.is_empty());
        let (view, pos) = far.snapshot.expect("a snapshot");
        assert_eq!(pos, head);
        assert_eq!(view.agents.len(), 6);
        // More bytes than the budget, though few events: a snapshot too.
        let heavy = hub.subscribe_within(Some(head - 2), 10, 1).unwrap();
        assert!(heavy.backlog.is_empty() && heavy.snapshot.is_some());
        // The snapshot/live boundary: the next event reaches the far subscriber live, once, right after the snapshot's position.
        hub.ingest(&mut s, &agent("n1", 7, "a7")).unwrap();
        let next = far.live.try_recv().unwrap();
        assert_eq!(next.pos, head + 1);
        assert!(far.live.try_recv().is_err());
    }

    #[test]
    fn a_client_away_for_more_than_the_replay_budget_resumes_from_a_snapshot() {
        let (hub, sup) = (hub(), support());
        let mut s = open_session(&hub, &sup, "n1");
        let first = hub.view().1;
        let total = MAX_REPLAY_EVENTS + 1;
        for i in 1..=total {
            hub.ingest(&mut s, &agent("n1", i, "a")).unwrap();
        }
        let sub = hub.subscribe(Some(first)).unwrap();
        assert!(
            sub.backlog.is_empty(),
            "replayed {} events",
            sub.backlog.len()
        );
        assert_eq!(sub.snapshot.map(|(_, pos)| pos), Some(hub.view().1));
    }

    #[test]
    fn resubscribing_from_a_position_replays_only_what_was_missed() {
        let (hub, sup) = (hub(), support());
        let mut s = open_session(&hub, &sup, "n1");
        for i in 1..=5 {
            hub.ingest(&mut s, &agent("n1", i, &format!("a{i}")))
                .unwrap();
        }
        let sub = hub.subscribe(Some(3)).unwrap();
        assert!(sub.snapshot.is_none());
        assert_eq!(
            sub.backlog.iter().map(|d| d.pos).collect::<Vec<_>>(),
            [4, 5]
        );
        // At the head: nothing to replay.
        assert!(hub.subscribe(Some(5)).unwrap().backlog.is_empty());
        // Ahead of the log (a restored hub): a snapshot instead of a lie.
        assert!(hub.subscribe(Some(99)).unwrap().snapshot.is_some());
    }

    /// Reviewer notes on #629: a second open over the same store keeps the replay barrier (it is recomputed from the stored history,
    /// never remembered) and adds nothing; a client that resumes from beyond the log's end gets a snapshot, with or without the barrier.
    #[test]
    fn a_second_open_over_a_history_with_a_barrier_keeps_it_and_a_resume_beyond_the_end_is_healed()
    {
        use agora_protocol::v1::AgentForgotten;
        let path =
            std::env::temp_dir().join(format!("agora-hub-barrier-{}.db", std::process::id()));
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
        let store = SqliteStore::open(&path).unwrap();
        let put = |producer: &str, seq: u64, body: Body| {
            let payload = Event {
                producer_id: producer.into(),
                seq,
                at_ms: seq,
                body: Some(body),
            }
            .encode_to_vec();
            store.append(producer, seq, seq, &payload).unwrap();
        };
        put(
            "n1",
            1,
            Body::NodeOnline(NodeOnline {
                node_id: "n1".into(),
                ..Default::default()
            }),
        );
        put(
            "n2",
            1,
            Body::NodeOnline(NodeOnline {
                node_id: "n2".into(),
                ..Default::default()
            }),
        );
        // A record stored before sessions refused cross-node facts: n2 forgetting n1's agent. Position 3 is the barrier.
        put(
            "n2",
            2,
            Body::AgentForgotten(AgentForgotten {
                agent_id: "n1/a".into(),
            }),
        );
        let first = Hub::open_at(store, 100).unwrap();
        let (first_view, first_end) = first.view();
        assert_eq!(first_view.distrusted_until, 3);
        assert!(first_end > 3, "opening recorded the machines offline");
        drop(first);

        let second = Hub::open_at(SqliteStore::open(&path).unwrap(), 200).unwrap();
        let (view, end) = second.view();
        assert_eq!(end, first_end, "opening again adds nothing");
        // A rebuild that meets a stored forged record puts the barrier at the end of the log it read (here the first open's end, which
        // includes the hub's own records of that open), so it only moves forward and never lets an older position replay.
        assert_eq!(
            view.distrusted_until, first_end,
            "recomputed from the history"
        );
        // At or before the barrier: a snapshot, however near the end.
        for after in [0, 1, 3, 4, end] {
            assert!(
                second.subscribe(Some(after)).unwrap().snapshot.is_some(),
                "after {after}"
            );
        }
        // Beyond the end: a snapshot, and the barrier does not turn that into a replay.
        for after in [end + 1, end + 1000, u64::MAX] {
            let sub = second.subscribe(Some(after)).unwrap();
            assert!(
                sub.snapshot.is_some() && sub.backlog.is_empty(),
                "after {after}"
            );
            assert_eq!(sub.snapshot.map(|(_, pos)| pos), Some(end));
        }
        drop(second);
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
    }

    #[test]
    fn a_slow_subscriber_is_dropped_and_does_not_stall_ingestion() {
        let (hub, sup) = (hub(), support());
        let mut s = open_session(&hub, &sup, "n1");
        let slow = hub.subscribe(None).unwrap();
        let fast = hub.subscribe(None).unwrap();
        assert_eq!(hub.subscriber_count(), 2);
        let n = SUBSCRIBER_BUFFER as u64 + 10;
        for i in 1..=n {
            hub.ingest(&mut s, &agent("n1", i, &format!("a{i}")))
                .unwrap();
            // The fast one keeps up.
            while fast.live.try_recv().is_ok() {}
        }
        // The slow one was dropped (its queue overflowed); the fast one stayed.
        assert_eq!(hub.subscriber_count(), 1);
        let got = std::iter::from_fn(|| slow.live.try_recv().ok()).count();
        assert_eq!(got, SUBSCRIBER_BUFFER);
        assert_eq!(hub.view().1, n);
    }

    /// What the view shows that a fact from n2 naming n1 would change: n1's card and agent, or the machine n2's entry is listed on.
    fn shown(body: &Body, v: &View) -> String {
        match body {
            Body::NodeOnline(_) => format!("n1/a running={}", v.agents["n1/a"].running),
            Body::NodeOffline(_) => format!("n1 online={}", v.nodes["n1"].online),
            Body::NodeResources(_) => format!("n1 agents_running={}", v.nodes["n1"].agents_running),
            Body::CheckoutDiscovered(_) => format!("n2/c on machine {}", v.checkouts["n2/c"].0),
            Body::AgentStarted(_) => format!("n2/a on machine {}", v.agents["n2/a"].node_id),
            Body::TerminalOpened(_) => format!("n2/t on machine {}", v.terminals["n2/t"].node_id),
            Body::WorkspaceRegistered(_) => {
                format!("n2/w on machine {}", v.workspaces["n2/w"].node_id)
            }
            _ => String::new(),
        }
    }

    /// Threat model T19: a node speaks for itself only. n2 sends each fact that carries a node id naming n1: every one is refused
    /// and changes nothing (n1's agent keeps running, its card and presence stay, nothing of n2's is listed on n1). The same facts
    /// naming n2 itself are accepted.
    #[test]
    fn a_node_cannot_speak_for_another_node() {
        let sup = support();
        let at = |seq: u64, body: Body| {
            Event {
                producer_id: "n2".into(),
                seq,
                at_ms: seq,
                body: Some(body),
            }
            .encode_to_vec()
        };
        let fresh = || {
            let hub = hub();
            let mut n1 = open_session(&hub, &sup, "n1");
            hub.ingest(&mut n1, &online("n1", 1)).unwrap();
            hub.ingest(&mut n1, &agent("n1", 2, "a")).unwrap();
            let mut n2 = open_session(&hub, &sup, "n2");
            hub.ingest(&mut n2, &online("n2", 1)).unwrap();
            drop((n1, n2));
            hub
        };
        let mut accepted = Vec::new();
        for body in crate::session::tests::naming("n1") {
            let hub = fresh();
            let mut n2 = open_session(&hub, &sup, "n2");
            let before = hub.view();
            match hub.ingest(&mut n2, &at(2, body.clone())) {
                Err(SessionError::Protocol(e)) => {
                    assert_eq!(e.code.as_str(), "protocol.invalid_message");
                    assert_eq!(hub.view(), before);
                    assert_eq!(hub.store().acked("n2").unwrap(), 1);
                }
                got => accepted.push(format!("{got:?}: {}", shown(&body, &hub.view().0))),
            }
        }
        assert!(accepted.is_empty(), "accepted naming n1: {accepted:#?}");
        // Control: the same facts naming the sender are its own word.
        for body in crate::session::tests::naming("n2") {
            let hub = fresh();
            let mut n2 = open_session(&hub, &sup, "n2");
            assert!(matches!(
                hub.ingest(&mut n2, &at(2, body)),
                Ok(Ingest::Applied { .. })
            ));
        }
    }

    /// ADR-0073, ADR-0053: forgetting a machine or an agent is the hub's own fact. A node that sends one about another machine or its
    /// agent is refused, and that agent stays listed and running.
    #[test]
    fn a_node_cannot_forget_another_machine_or_its_agent() {
        let sup = support();
        let mut accepted = Vec::new();
        for body in [
            Body::NodeForgotten(agora_protocol::v1::NodeForgotten {
                node_id: "n1".into(),
                refused_return: true,
            }),
            Body::AgentForgotten(agora_protocol::v1::AgentForgotten {
                agent_id: "n1/a".into(),
            }),
        ] {
            let hub = hub();
            let mut n1 = open_session(&hub, &sup, "n1");
            hub.ingest(&mut n1, &online("n1", 1)).unwrap();
            hub.ingest(&mut n1, &agent("n1", 2, "a")).unwrap();
            let mut n2 = open_session(&hub, &sup, "n2");
            hub.ingest(&mut n2, &online("n2", 1)).unwrap();
            let before = hub.view();
            let forget = Event {
                producer_id: "n2".into(),
                seq: 2,
                at_ms: 2,
                body: Some(body.clone()),
            }
            .encode_to_vec();
            match hub.ingest(&mut n2, &forget) {
                Err(SessionError::Protocol(e)) => {
                    assert_eq!(e.code.as_str(), "protocol.invalid_message");
                    assert_eq!(hub.view(), before);
                    assert_eq!(hub.store().acked("n2").unwrap(), 1);
                }
                got => {
                    let (v, _) = hub.view();
                    let (n, a) = (&v.nodes["n1"], &v.agents["n1/a"]);
                    accepted.push(format!(
                        "{body:?} {got:?}: n1 forgotten={}, n1/a forgotten={} running={}",
                        n.forgotten, a.forgotten, a.running
                    ));
                }
            }
        }
        assert!(accepted.is_empty(), "accepted: {accepted:#?}");
    }

    #[test]
    fn a_dropped_connection_is_recorded_as_a_hub_event_and_survives_restart() {
        let (hub, sup) = (hub(), support());
        let mut s = open_session(&hub, &sup, "n1");
        hub.ingest(&mut s, &online("n1", 1)).unwrap();
        let sub = hub.subscribe(None).unwrap();
        hub.node_disconnected("n1", 50).unwrap();
        assert!(!hub.view().0.nodes["n1"].online);
        let d = sub.live.try_recv().unwrap();
        assert_eq!(d.event.producer_id, RESERVED_HUB_ID);
        // A second drop gets the next hub seq, and the log stays contiguous for producer "hub".
        hub.node_disconnected("n1", 60).unwrap();
        assert_eq!(hub.store().acked(RESERVED_HUB_ID).unwrap(), 2);
        // Reopening on the same store rebuilds the same view and continues the hub's numbering.
        let live_view = hub.view();
        let store = SqliteStore::open_in_memory().unwrap();
        for e in hub.store().events_after(0, 100).unwrap() {
            store
                .append(&e.producer, e.seq, e.at_ms, &e.payload)
                .unwrap();
        }
        let again = Hub::open(store).unwrap();
        assert_eq!(again.view(), live_view);
        again.node_disconnected("n1", 70).unwrap();
        assert_eq!(again.store().acked(RESERVED_HUB_ID).unwrap(), 3);
    }

    /// ADR-0070: right after the hub opens no connection exists, so a node the log leaves online (the hub was killed with its
    /// connection open, or only an old `NodeOnline` says so) is recorded offline, once, with "last seen" from the log.
    #[test]
    fn opening_the_hub_records_nodes_the_log_leaves_online_as_offline_once() {
        use agora_protocol::v1::NodeResources;
        let store = SqliteStore::open_in_memory().unwrap();
        let put = |producer: &str, seq: u64, at_ms: u64, body: Body| {
            let e = Event {
                producer_id: producer.into(),
                seq,
                at_ms,
                body: Some(body),
            };
            store
                .append(producer, seq, at_ms, &e.encode_to_vec())
                .unwrap();
        };
        let link = |node: &str, up: bool, reason: NodeLinkReason, generation, heard| {
            Body::NodeLink(NodeLink {
                node_id: node.into(),
                up,
                reason: reason as i32,
                generation,
                last_heard_ms: heard,
            })
        };
        let started = |node: &str| {
            Body::NodeOnline(NodeOnline {
                node_id: node.into(),
                hostname: "h".into(),
                os: "linux".into(),
                build: "b".into(),
                binary: String::new(),
            })
        };
        // "up": its connection (generation 2) was open when the hub was killed; it was last heard at 500.
        put("up", 1, 10, started("up"));
        put(
            "hub",
            1,
            11,
            link("up", true, NodeLinkReason::Opened, 1, 11),
        );
        put(
            "hub",
            2,
            20,
            link("up", false, NodeLinkReason::Closed, 1, 19),
        );
        put(
            "hub",
            3,
            30,
            link("up", true, NodeLinkReason::Opened, 2, 30),
        );
        put(
            "up",
            2,
            500,
            Body::NodeResources(NodeResources {
                node_id: "up".into(),
                ..NodeResources::default()
            }),
        );
        // "down": its last fact is offline already.
        put("down", 1, 40, started("down"));
        put(
            "hub",
            4,
            41,
            link("down", true, NodeLinkReason::Opened, 1, 41),
        );
        put(
            "hub",
            5,
            60,
            link("down", false, NodeLinkReason::Silent, 1, 45),
        );
        // "old": seen only through a `NodeOnline` from before ADR-0070.
        put("old", 1, 70, started("old"));
        let before = store.events_after(0, 100).unwrap().len();

        let hub = Hub::open_at(store, 9_000).unwrap();
        let (view, _) = hub.view();
        for (node, seen) in [("up", 500), ("down", 45), ("old", 70)] {
            assert!(!view.nodes[node].online, "{node} is offline");
            assert_eq!(view.nodes[node].last_event_ms, seen, "{node} last seen");
        }
        let added: Vec<Event> = hub
            .store()
            .events_after(0, 100)
            .unwrap()
            .iter()
            .skip(before)
            .filter_map(decode)
            .map(|d| d.event)
            .collect();
        assert!(added
            .iter()
            .all(|e| e.producer_id == RESERVED_HUB_ID && e.at_ms == 9_000));
        // The hub's numbering continues where the log left it.
        assert_eq!(
            added.iter().map(|e| e.seq).collect::<Vec<_>>(),
            [6, 7, 8, 9]
        );
        let mut facts: Vec<(String, Option<(u64, u64)>)> = added
            .into_iter()
            .map(|e| match e.body {
                Some(Body::NodeOffline(o)) => (o.node_id, None),
                Some(Body::NodeLink(l)) => {
                    assert!(!l.up);
                    assert_eq!(l.reason, NodeLinkReason::Closed as i32);
                    (l.node_id, Some((l.generation, l.last_heard_ms)))
                }
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        facts.sort();
        assert_eq!(
            facts,
            [
                ("old".to_string(), None),
                ("old".to_string(), Some((0, 70))),
                ("up".to_string(), None),
                ("up".to_string(), Some((2, 500))),
            ]
        );
        // A rebuilt view agrees with the live one, and opening again records nothing.
        let live = hub.view();
        let store = SqliteStore::open_in_memory().unwrap();
        for e in hub.store().events_after(0, 100).unwrap() {
            store
                .append(&e.producer, e.seq, e.at_ms, &e.payload)
                .unwrap();
        }
        let again = Hub::open_at(store, 10_000).unwrap();
        assert_eq!(again.view(), live);
        assert_eq!(again.store().acked(RESERVED_HUB_ID).unwrap(), 9);
        // The next connection of "up" is a newer generation and brings it back online.
        let (_reg, _cmds, generation) = again.node_link_opened("up", Vec::new(), 10_001).unwrap();
        assert_eq!(generation, 3);
        assert!(again.view().0.nodes["up"].online);
    }

    #[test]
    fn concurrent_nodes_and_subscribers_see_every_event_once_in_order() {
        let hub = std::sync::Arc::new(hub());
        let sup = support();
        let per_node = 200u64;
        // A subscriber joins while ingestion is already running.
        let writers: Vec<_> = (0..3)
            .map(|n| {
                let hub = hub.clone();
                let sup = sup.clone();
                std::thread::spawn(move || {
                    let node = format!("n{n}");
                    let mut s = open_session(&hub, &sup, &node);
                    for i in 1..=per_node {
                        hub.ingest(&mut s, &agent(&node, i, &format!("a{i}")))
                            .unwrap();
                    }
                })
            })
            .collect();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let sub = hub.subscribe(None).unwrap();
        let (_, snap_pos) = sub.snapshot.clone().unwrap();
        for w in writers {
            w.join().unwrap();
        }
        let positions: Vec<u64> = std::iter::from_fn(|| sub.live.try_recv().ok())
            .map(|d| d.pos)
            .collect();
        // Live events continue exactly after the snapshot, one by one, through the end of the log.
        let total = 3 * per_node;
        assert_eq!(positions, (snap_pos + 1..=total).collect::<Vec<_>>());
    }

    #[test]
    fn nodes_cannot_take_the_hub_producer_id() {
        let ca = crate::ca::Ca::generate("ca").unwrap();
        let (_, csr) = crate::ca::new_device_request().unwrap();
        assert!(ca.issue_node(RESERVED_HUB_ID, &csr).is_err());
    }

    #[test]
    fn ingest_publishes_bounded_quota_timestamp_matching_store() {
        let hub = Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap();
        let sup = Support {
            generations: vec![1],
            capabilities: vec![],
            required: vec![],
        };
        let mut session = open_session(&hub, &sup, "n1");
        let ev = Event {
            producer_id: "n1".into(),
            seq: 1,
            at_ms: 0,
            body: Some(Body::AgentQuotaObserved(
                agora_protocol::v1::AgentQuotaObserved {
                    node_id: "n1".into(),
                    tool: "claude".into(),
                    window: "5h".into(),
                    observed_at_ms: u64::MAX - 1,
                    used_percent: 20,
                    ..Default::default()
                },
            )),
        };
        hub.ingest(&mut session, &ev.encode_to_vec()).unwrap();
        let key = ("n1".to_string(), "claude".to_string(), "5h".to_string());
        let live_ts = hub.view().0.quota[&key].observed_at_ms;
        let rebuilt_ts = crate::projection::View::rebuild(hub.store())
            .unwrap()
            .0
            .quota[&key]
            .observed_at_ms;
        assert_eq!(live_ts, rebuilt_ts);
        assert!(live_ts < u64::MAX - 1);
    }

    const ROOT_0115: &str = "db6f7d0123456789abcdef0123456789abcdef01";

    /// ADR-0115: an older node's checkout and agent in `project` (in whatever case it spells it).
    fn in_project(node: &str, seq: u64, project: &str) -> Vec<Vec<u8>> {
        let ev = |seq: u64, body: Body| {
            Event {
                producer_id: node.into(),
                seq,
                at_ms: seq,
                body: Some(body),
            }
            .encode_to_vec()
        };
        vec![
            ev(
                seq,
                Body::CheckoutDiscovered(agora_protocol::v1::CheckoutDiscovered {
                    checkout_id: "c".into(),
                    node_id: node.into(),
                    project_id: project.into(),
                    path: "/src/agora".into(),
                }),
            ),
            ev(
                seq + 1,
                Body::AgentStarted(AgentStarted {
                    agent_id: "a".into(),
                    project_id: project.into(),
                    node_id: node.into(),
                    worktree_id: "w".into(),
                    kind: "shell".into(),
                    level: 2,
                    ..Default::default()
                }),
            ),
        ]
    }

    fn board_call_0115(project: &str, tool: &str, args: &str) -> agora_protocol::v1::BoardCall {
        agora_protocol::v1::BoardCall {
            from_agent: "a".into(),
            project_id: project.into(),
            tool: tool.into(),
            arguments_json: args.into(),
            ..Default::default()
        }
    }

    /// ADR-0115: `old` (an older node, its clone's remote `…/ersinkoc/Agora`) and `new` (a newer node, which folds `…/ersinkoc/agora`).
    fn two_spellings(hub: &Hub) -> (String, String) {
        let sup = support();
        let old = format!("github.com/ersinkoc/Agora#{ROOT_0115}");
        let folded = format!("github.com/ersinkoc/agora#{ROOT_0115}");
        for (node, project) in [("old", &old), ("new", &folded)] {
            let mut s = open_session(hub, &sup, node);
            hub.ingest(&mut s, &online(node, 1)).unwrap();
            for ev in in_project(node, 2, project) {
                hub.ingest(&mut s, &ev).unwrap();
            }
        }
        (old, folded)
    }

    /// ADR-0115: a node that spells a GitHub repository in another case is in the same project as one that spells it folded, in the
    /// hub's view, its rebuilt view, board membership and the board; a command about the project reaches each in its own spelling.
    #[test]
    fn an_older_nodes_cased_project_id_is_one_project_with_a_newer_nodes() {
        let (hub, path) = hub_on_file("adr0115-one-project");
        let (old, folded) = two_spellings(&hub);
        let check = |view: &View| {
            assert_eq!(view.checkouts["old/c"].1, folded);
            assert_eq!(view.checkouts["new/c"].1, folded);
            assert_eq!(view.agents["old/a"].project_id, folded);
            assert_eq!(view.agents["new/a"].project_id, folded);
            assert_eq!(crate::board::nodes_with(view, &folded), ["new", "old"]);
            assert!(crate::board::nodes_with(view, &old).is_empty());
            assert_eq!(view.spelled_for("old", &folded), old);
            assert_eq!(view.spelled_for("new", &folded), folded);
        };
        check(&hub.view().0);
        check(&View::rebuild(hub.store()).unwrap().0);
        // One shared project: both nodes are asked to join it, by its folded id.
        let mut asked = hub.board_membership(4);
        asked.sort();
        assert_eq!(
            asked,
            [
                ("new".to_string(), folded.clone()),
                ("old".to_string(), folded.clone())
            ]
        );
        hub.board_join_settled("old", &folded);
        hub.board_join_settled("new", &folded);
        // The older node's board call names the project in its case; the newer node sees the task it made.
        let made = hub.board_call(
            "old",
            &board_call_0115(&old, "task_create", r#"{"title":"ship it"}"#),
            5,
        );
        assert!(
            matches!(
                made.verdict,
                crate::board::Verdict::Done {
                    is_error: false,
                    ..
                }
            ),
            "{:?}",
            made.verdict
        );
        let listed = hub.board_call("new", &board_call_0115(&folded, "task_list", "{}"), 6);
        match listed.verdict {
            crate::board::Verdict::Done { result, .. } => {
                assert!(result.to_string().contains("ship it"), "{result}");
            }
            other => panic!("{other:?}"),
        }
        let view = hub.view().0;
        assert!(view.tasks.values().all(|t| t.project_id == folded));
        assert!(!view.tasks.is_empty());
        // A command about the project, as a window sends it (folded), reaches each node in its own spelling.
        let dispatch = |node: &str| {
            let req = agora_protocol::v1::CommandRequest {
                command: Some(agora_protocol::v1::command_request::Command::TaskDispatch(
                    agora_protocol::v1::TaskDispatch {
                        project_id: folded.clone(),
                        task_id: "t".into(),
                        agent_id: "a".into(),
                        by_user: String::new(),
                    },
                )),
                ..Default::default()
            };
            match hub.spelled_for(node, req).command {
                Some(agora_protocol::v1::command_request::Command::TaskDispatch(d)) => d.project_id,
                other => panic!("{other:?}"),
            }
        };
        assert_eq!(dispatch("old"), old);
        assert_eq!(dispatch("new"), folded);
        let join = hub.spelled_for(
            "old",
            agora_protocol::v1::CommandRequest {
                command: Some(agora_protocol::v1::command_request::Command::BoardJoin(
                    agora_protocol::v1::BoardJoin {
                        project_id: folded.clone(),
                    },
                )),
                ..Default::default()
            },
        );
        assert!(matches!(
            join.command,
            Some(agora_protocol::v1::command_request::Command::BoardJoin(j)) if j.project_id == old
        ));
        // Stored events keep what the node sent.
        let stored = hub.store().events_after(0, 100).unwrap();
        assert!(stored.iter().any(|stored| {
            Event::decode(stored.payload.as_slice()).is_ok_and(
                |e| matches!(e.body, Some(Body::CheckoutDiscovered(c)) if c.project_id == old),
            )
        }));
        drop(hub);
        remove_db(&path);
    }

    /// ADR-0115: once the older node updates and reports the folded id, the hub forgets its old spelling; a node that never spelled
    /// it otherwise is sent the id as given, and another host keeps its case (two projects).
    #[test]
    fn a_node_that_updates_is_sent_the_folded_id_and_other_hosts_keep_their_case() {
        let hub = hub();
        let (_, folded) = two_spellings(&hub);
        let sup = support();
        let mut s = open_session(&hub, &sup, "old");
        for ev in in_project("old", 4, &folded) {
            hub.ingest(&mut s, &ev).unwrap();
        }
        assert_eq!(hub.view().0.spelled_for("old", &folded), folded);
        assert!(hub.view().0.spellings.is_empty());
        let mut s = open_session(&hub, &sup, "self-hosted");
        hub.ingest(&mut s, &online("self-hosted", 1)).unwrap();
        for ev in in_project(
            "self-hosted",
            2,
            &format!("git.example.org/O/R#{ROOT_0115}"),
        ) {
            hub.ingest(&mut s, &ev).unwrap();
        }
        let view = hub.view().0;
        assert_eq!(
            view.checkouts["self-hosted/c"].1,
            format!("git.example.org/O/R#{ROOT_0115}")
        );
        assert!(
            crate::board::nodes_with(&view, &format!("git.example.org/o/r#{ROOT_0115}")).is_empty()
        );
    }

    /// ADR-0115: a board an older hub stored under the project's spelling in another case is the folded project's board after the
    /// update, and is stored again under the folded id (the old record stays for a downgrade).
    #[test]
    fn a_board_stored_under_an_older_spelling_is_found() {
        let (hub, path) = hub_on_file("adr0115-old-board");
        let (old, folded) = two_spellings(&hub);
        hub.store()
            .set_setting(
                &format!("board.{old}"),
                br#"{"next": 2, "tasks": [{"id": "t1", "title": "kept from before", "detail": "", "depends_on": [], "done": false, "owner": null, "summary": ""}], "leases": []}"#,
            )
            .unwrap();
        hub.board_join_settled("old", &folded);
        hub.board_join_settled("new", &folded);
        let listed = hub.board_call("new", &board_call_0115(&folded, "task_list", "{}"), 6);
        match listed.verdict {
            crate::board::Verdict::Done { result, .. } => {
                assert!(result.to_string().contains("kept from before"), "{result}");
            }
            other => panic!("{other:?}"),
        }
        let again = hub
            .store()
            .setting(&format!("board.{folded}"))
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&again).contains("kept from before"));
        assert!(hub
            .store()
            .setting(&format!("board.{old}"))
            .unwrap()
            .is_some());
        drop(hub);
        remove_db(&path);
    }

    /// ADR-0115: an older hub kept a board per spelling. Every one is merged into the folded project's board once (an id taken twice
    /// gets a new one, its dependency follows), stored under the folded id, and a restarted hub does not add them again.
    #[test]
    fn every_board_stored_under_an_older_spelling_is_merged_once() {
        let (hub, path) = hub_on_file("adr0115-many-boards");
        let (old, folded) = two_spellings(&hub);
        let upper = format!("github.com/ERSINKOC/AGORA#{ROOT_0115}");
        let sup = support();
        let mut s = open_session(&hub, &sup, "older");
        hub.ingest(&mut s, &online("older", 1)).unwrap();
        for ev in in_project("older", 2, &upper) {
            hub.ingest(&mut s, &ev).unwrap();
        }
        let task = |id: &str, title: &str, deps: &[&str]| serde_json::json!({"id": id, "title": title, "detail": "", "depends_on": deps, "done": false, "owner": null, "summary": ""});
        let board = |tasks: Vec<serde_json::Value>| {
            serde_json::json!({"next": 3, "tasks": tasks, "leases": []}).to_string()
        };
        let put = |spelling: &str, text: String| {
            hub.store()
                .set_setting(&format!("board.{spelling}"), text.as_bytes())
                .unwrap();
        };
        put(&folded, board(vec![task("t-1", "folded one", &[])]));
        put(&old, board(vec![task("t-1", "cased one", &[])]));
        put(
            &upper,
            board(vec![
                task("t-1", "upper one", &[]),
                task("t-2", "upper two", &["t-1"]),
            ]),
        );
        for n in ["old", "new", "older"] {
            hub.board_join_settled(n, &folded);
        }
        let list = |hub: &Hub| -> serde_json::Value {
            match hub
                .board_call("new", &board_call_0115(&folded, "task_list", "{}"), 6)
                .verdict
            {
                crate::board::Verdict::Done { result, .. } => {
                    serde_json::from_str(result["content"][0]["text"].as_str().unwrap_or("null"))
                        .unwrap()
                }
                other => panic!("{other:?}"),
            }
        };
        let titles = |v: &serde_json::Value| -> Vec<String> {
            let mut t: Vec<String> = v
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["title"].as_str().unwrap().to_string())
                .collect();
            t.sort();
            t
        };
        let first = list(&hub);
        assert_eq!(
            titles(&first),
            ["cased one", "folded one", "upper one", "upper two"],
            "{first}"
        );
        let ids: std::collections::BTreeSet<&str> = first
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["task_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids.len(), 4, "{first}");
        let two = first
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["title"] == "upper two")
            .unwrap();
        let one = first
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["title"] == "upper one")
            .unwrap();
        assert_eq!(two["depends_on"][0], one["task_id"], "{first}");
        assert!(hub
            .store()
            .setting(&format!("board.{old}"))
            .unwrap()
            .is_some());
        drop(hub);
        // Restarted: the merged board is read back under the folded id, and the older ones are not added again.
        let hub = Hub::open(SqliteStore::open(&path).unwrap()).unwrap();
        let mut links = Vec::new();
        for n in ["old", "new", "older"] {
            links.push(hub.node_link_opened(n, Vec::new(), 7).unwrap());
            hub.board_join_settled(n, &folded);
        }
        let again = list(&hub);
        assert_eq!(titles(&again), titles(&first), "{again}");
        drop(hub);
        remove_db(&path);
    }

    /// ADR-0115 board, as an older hub stored it.
    fn board_text(tasks: &[(&str, &str, Option<&str>)]) -> String {
        let tasks: Vec<serde_json::Value> = tasks
            .iter()
            .map(|(id, title, owner)| serde_json::json!({"id": id, "title": title, "detail": "", "depends_on": [], "done": false, "owner": owner, "summary": ""}))
            .collect();
        serde_json::json!({"next": 3, "tasks": tasks, "leases": []}).to_string()
    }

    /// The titles `task_list` gives for `project` on `hub`, sorted.
    fn listed_titles(hub: &Hub, project: &str) -> Vec<String> {
        match hub
            .board_call("new", &board_call_0115(project, "task_list", "{}"), 6)
            .verdict
        {
            crate::board::Verdict::Done { result, .. } => {
                let v: serde_json::Value =
                    serde_json::from_str(result["content"][0]["text"].as_str().unwrap_or("null"))
                        .unwrap();
                let mut t: Vec<String> = v
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|t| t["title"].as_str().unwrap().to_string())
                    .collect();
                t.sort();
                t
            }
            other => panic!("{other:?}"),
        }
    }

    /// The ADR-0115 merge scenario of the tests below: the folded board's `t-1` and `t-2`, and the older spelling's `t-1`, claimed by
    /// `old/a`. The hub is opened again on its file, so the agents are still running and their nodes did not start again since (a
    /// node that did lost its agents, and their claims go back as the board loads); the machines in `linked` connect.
    fn merge_scenario(
        name: &str,
        linked: &[&str],
    ) -> (
        Hub,
        std::path::PathBuf,
        String,
        Vec<crate::router::Registration>,
    ) {
        let (hub, path) = hub_on_file(name);
        let (old, folded) = two_spellings(&hub);
        let put = |spelling: &str, text: String| {
            hub.store()
                .set_setting(&format!("board.{spelling}"), text.as_bytes())
                .unwrap();
        };
        put(
            &folded,
            board_text(&[("t-1", "folded one", None), ("t-2", "folded two", None)]),
        );
        put(&old, board_text(&[("t-1", "cased one", Some("old/a"))]));
        drop(hub);
        let hub = Hub::open(SqliteStore::open(&path).unwrap()).unwrap();
        let mut links = Vec::new();
        for n in linked {
            links.push(hub.node_link_opened(n, Vec::new(), 5).unwrap().0);
        }
        for n in ["old", "new"] {
            hub.board_join_settled(n, &folded);
        }
        (hub, path, folded, links)
    }

    /// #1150 (ADR-0115 notes 1 and 2): merging an older hub's boards tells the window of each task given a new id and of the task
    /// that kept the id (it showed both boards' `t-1` as one), and tells the owner of a moved task its new id (`task_moved`, as after
    /// a join), once.
    #[test]
    fn a_board_merge_restates_the_tasks_it_renamed_and_tells_their_owners() {
        let (hub, path, folded, links) = merge_scenario("adr0115-merge-events", &["old", "new"]);
        let first = hub.board_call("new", &board_call_0115(&folded, "task_list", "{}"), 6);
        let view = hub.view().0;
        let task = |title: &str| {
            view.tasks
                .iter()
                .find(|(_, t)| t.title == title)
                .map(|(k, t)| (k.clone(), t.owner.clone()))
        };
        assert_eq!(
            task("folded one"),
            Some((format!("{folded}/t-1"), String::new())),
            "{:?}",
            view.tasks
        );
        let (moved_key, owner) = task("cased one").expect("the moved task is shown");
        assert_eq!(owner, "old/a", "its owner keeps it under its new id");
        let moved = moved_key
            .strip_prefix(&format!("{folded}/"))
            .unwrap()
            .to_string();
        assert_ne!(moved, "t-1");
        assert!(
            task("folded two").is_none(),
            "a task whose id no other board had is not restated"
        );
        assert_eq!(
            first.notices,
            [crate::board::Notice {
                to_node: "old".into(),
                to_agent: "a".into(),
                body: serde_json::json!({"board": "task_moved", "from": "t-1", "to": moved})
                    .to_string(),
            }]
        );
        let again = hub.board_call("new", &board_call_0115(&folded, "task_list", "{}"), 7);
        assert!(again.notices.is_empty());
        assert!(hub.take_owed_notices("old").is_empty());
        drop(links);
        drop(hub);
        remove_db(&path);
    }

    /// #1150 (review of #1223, H2): after a hub start, the merge runs at the first board call, often before every machine is back. A
    /// notice for an agent whose machine is not connected is kept and given when it connects, once; nothing is sent to it before.
    #[test]
    fn a_board_merge_notice_for_a_machine_not_connected_waits_for_it() {
        let (hub, path, folded, mut links) = merge_scenario("adr0115-merge-owed", &["new"]);
        let first = hub.board_call("new", &board_call_0115(&folded, "task_list", "{}"), 6);
        assert!(first.notices.is_empty(), "{:?}", first.notices);
        links.push(hub.node_link_opened("old", Vec::new(), 7).unwrap().0);
        let owed = hub.take_owed_notices("old");
        assert_eq!(owed.len(), 1, "{owed:?}");
        assert_eq!(
            (owed[0].to_node.as_str(), owed[0].to_agent.as_str()),
            ("old", "a")
        );
        assert!(owed[0].body.contains("task_moved"), "{}", owed[0].body);
        assert!(hub.take_owed_notices("old").is_empty(), "given once");
        drop(links);
        drop(hub);
        remove_db(&path);
    }

    /// #1150 (ADR-0115 note 3): the merged board and its `board-merged.<spelling>` markers are stored together. When the store
    /// refuses the markers, the folded board is used alone (nothing of the merge is stored), a change made meanwhile is kept, and the
    /// next start merges once: no task is added twice, none is lost.
    #[test]
    fn a_board_merge_whose_markers_are_refused_never_adds_tasks_twice() {
        let (hub, path) = hub_on_file("adr0115-merge-markers");
        let (old, folded) = two_spellings(&hub);
        for (spelling, title) in [(&folded, "folded one"), (&old, "cased one")] {
            hub.store()
                .set_setting(
                    &format!("board.{spelling}"),
                    board_text(&[("t-1", title, None)]).as_bytes(),
                )
                .unwrap();
        }
        let other = rusqlite::Connection::open(&path).unwrap();
        other
            .execute_batch(
                "CREATE TRIGGER no_markers BEFORE INSERT ON settings WHEN NEW.key LIKE 'board-merged.%'
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .unwrap();
        for n in ["old", "new"] {
            hub.board_join_settled(n, &folded);
        }
        assert_eq!(listed_titles(&hub, &folded), ["folded one"]);
        let made = hub.board_call(
            "new",
            &board_call_0115(&folded, "task_create", r#"{"title": "made meanwhile"}"#),
            7,
        );
        assert!(
            matches!(
                made.verdict,
                crate::board::Verdict::Done {
                    is_error: false,
                    ..
                }
            ),
            "{:?}",
            made.verdict
        );
        other.execute_batch("DROP TRIGGER no_markers;").unwrap();
        drop(other);
        drop(hub);
        for _ in 0..2 {
            let hub = Hub::open(SqliteStore::open(&path).unwrap()).unwrap();
            for n in ["old", "new"] {
                let _ = hub.node_link_opened(n, Vec::new(), 8).unwrap();
                hub.board_join_settled(n, &folded);
            }
            assert_eq!(
                listed_titles(&hub, &folded),
                ["cased one", "folded one", "made meanwhile"]
            );
        }
        remove_db(&path);
    }

    /// A hub on a file, another connection to the same file (what another process is: `agora hub login`, the front door, `agora hub
    /// code`), and a cleanup.
    fn hub_on_file(name: &str) -> (Hub, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("agora-hub-{name}-{}.db", std::process::id()));
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
        (Hub::open(SqliteStore::open(&path).unwrap()).unwrap(), path)
    }

    fn remove_db(path: &std::path::Path) {
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
    }

    /// #957: a machine that connects while another process writes the hub's database is shown online. The record of its connection
    /// read before it wrote, in a deferred transaction, so SQLite answered "database is locked" at once instead of waiting; the
    /// failure was dropped, and the machine stayed connected but offline in every view, with nothing to correct it (seen as "the node
    /// never came back online": the test's own `agora hub login` wrote the database while the node reconnected).
    #[test]
    fn a_machine_that_connects_while_another_process_writes_is_shown_online() {
        let (hub, path) = hub_on_file("presence-busy");
        let other = rusqlite::Connection::open(&path).unwrap();
        other
            .execute_batch("BEGIN IMMEDIATE; INSERT OR REPLACE INTO settings (key, value) VALUES ('other', x'00');")
            .unwrap();
        let commit = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            other.execute_batch("COMMIT;").unwrap();
        });
        let opened = hub.node_link_opened("n1", Vec::new(), 1);
        commit.join().unwrap();
        assert!(opened.is_ok());
        assert!(hub.view().0.nodes.get("n1").is_some_and(|n| n.online));
        drop(hub);
        remove_db(&path);
    }

    /// #957: a connection whose opening the hub cannot record is not live: the error comes back (the server ends the connection),
    /// the router does not have the node, and the node's next connection, once the store works again, is recorded online. Before,
    /// the router took it and the failure was dropped, so the machine was connected but shown offline for as long as it stayed.
    #[test]
    fn a_connection_whose_opening_cannot_be_recorded_is_not_kept() {
        let (hub, path) = hub_on_file("presence-refused");
        let other = rusqlite::Connection::open(&path).unwrap();
        other
            .execute_batch("CREATE TRIGGER no_events BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT, 'full'); END;")
            .unwrap();
        assert!(hub.node_link_opened("n1", Vec::new(), 1).is_err());
        assert!(!hub.router().is_connected("n1"));
        assert!(!hub.view().0.nodes.get("n1").is_some_and(|n| n.online));
        other.execute_batch("DROP TRIGGER no_events;").unwrap();
        let (_reg, _cmds, generation) = hub.node_link_opened("n1", Vec::new(), 2).unwrap();
        assert_eq!(generation, 1);
        assert!(hub.router().is_connected("n1"));
        assert!(hub.view().0.nodes["n1"].online);
        drop(hub);
        remove_db(&path);
    }

    const REFUSE_EVENTS: &str =
        "CREATE TRIGGER no_events BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT, 'full'); END;";

    /// The presence facts in the log about `node`, as `(up, reason, generation)` (`NodeOffline` is `(false, -1, 0)`).
    fn presence(hub: &Hub, node: &str) -> Vec<(bool, i32, u64)> {
        hub.store()
            .events_after(0, 10_000)
            .unwrap()
            .iter()
            .filter_map(decode)
            .filter_map(|d| match d.event.body {
                Some(Body::NodeLink(l)) if l.node_id == node => {
                    Some((l.up, l.reason, l.generation))
                }
                Some(Body::NodeOffline(o)) if o.node_id == node => Some((false, -1, 0)),
                _ => None,
            })
            .collect()
    }

    /// #996: the end of a connection the store refused to record is kept, not dropped, and written once the store works again,
    /// with no reconnect: the view, a live subscriber and a rebuilt view all show the machine offline, and it can be forgotten.
    /// Before, the failure was dropped and the machine that had left stayed online until it came back or the hub restarted.
    #[test]
    fn a_connection_whose_end_cannot_be_recorded_is_recorded_once_the_store_works() {
        let (hub, path) = hub_on_file("presence-end-refused");
        let (reg, _cmds, generation) = hub.node_link_opened("n1", Vec::new(), 1).unwrap();
        let sub = hub.subscribe(None).unwrap();
        let other = rusqlite::Connection::open(&path).unwrap();
        other.execute_batch(REFUSE_EVENTS).unwrap();
        assert!(hub.node_link_closed(reg, generation, false, 5, 6));
        assert!(!hub.router().is_connected("n1"));
        assert!(hub.has_unrecorded());
        assert!(hub.record_unrecorded().is_err());
        assert!(hub.has_unrecorded());
        other.execute_batch("DROP TRIGGER no_events;").unwrap();
        hub.record_unrecorded().unwrap();
        assert!(!hub.has_unrecorded());
        let (view, _) = hub.view();
        assert!(!view.nodes["n1"].online);
        assert_eq!(view.nodes["n1"].last_event_ms, 5);
        assert!(!View::rebuild(hub.store()).unwrap().0.nodes["n1"].online);
        let live: Vec<_> = std::iter::from_fn(|| sub.live.try_recv().ok())
            .filter_map(|d| match d.event.body {
                Some(Body::NodeLink(l)) => Some((l.up, l.reason, l.generation)),
                Some(Body::NodeOffline(_)) => Some((false, -1, 0)),
                _ => None,
            })
            .collect();
        assert_eq!(
            live,
            [(false, -1, 0), (false, NodeLinkReason::Closed as i32, 1)]
        );
        hub.forget_node("n1", false, 7).unwrap();
        drop(hub);
        remove_db(&path);
    }

    /// #996: when only the second of a connection's two end facts is refused, only that one is kept: the first is not written twice.
    #[test]
    fn only_the_end_facts_the_store_refused_are_written_later() {
        let (hub, path) = hub_on_file("presence-end-partial");
        let (reg, _cmds, generation) = hub.node_link_opened("n1", Vec::new(), 1).unwrap();
        let other = rusqlite::Connection::open(&path).unwrap();
        // The log holds the opening; `NodeOffline` is the second event and goes in, `NodeLink` would be the third.
        other
            .execute_batch("CREATE TRIGGER no_events BEFORE INSERT ON events WHEN (SELECT COUNT(*) FROM events) >= 2 BEGIN SELECT RAISE(ABORT, 'full'); END;")
            .unwrap();
        assert!(hub.node_link_closed(reg, generation, false, 5, 6));
        assert!(!hub.view().0.nodes["n1"].online);
        assert!(hub.has_unrecorded());
        other.execute_batch("DROP TRIGGER no_events;").unwrap();
        hub.record_unrecorded().unwrap();
        assert_eq!(
            presence(&hub, "n1"),
            [
                (true, NodeLinkReason::Opened as i32, 1),
                (false, -1, 0),
                (false, NodeLinkReason::Closed as i32, 1),
            ]
        );
        drop(hub);
        remove_db(&path);
    }

    /// ADR-0112 Amendment A (review of #1205): a link whose opening the store refuses does not keep the earlier link's agreement on
    /// the hub-wide view; the machine is left out until a link is recorded.
    #[test]
    fn a_refused_link_does_not_keep_the_earlier_agreement_on_the_fleet_view() {
        let (hub, path) = hub_on_file("fleet-refused-link");
        let fleet = vec![agora_protocol::handshake::FLEET_STATUS_V1.to_string()];
        let _first = hub.node_link_opened("n1", fleet.clone(), 1).unwrap();
        assert!(hub.lock().fleet_agreed.contains("n1"));
        let other = rusqlite::Connection::open(&path).unwrap();
        other.execute_batch(REFUSE_EVENTS).unwrap();
        assert!(hub.node_link_opened("n1", fleet.clone(), 2).is_err());
        assert!(!hub.lock().fleet_agreed.contains("n1"));
        other.execute_batch("DROP TRIGGER no_events;").unwrap();
        let _again = hub.node_link_opened("n1", fleet, 3).unwrap();
        assert!(hub.lock().fleet_agreed.contains("n1"));
        drop(hub);
        remove_db(&path);
    }

    /// #996: a board call writes the ends the store refused before it decides anything, so the board sees a machine that left as
    /// offline (never hands work to it) and its events come after that record. While the store still refuses, the call is answered
    /// `internal` (`board.store`) and the end stays waiting.
    #[test]
    fn a_board_call_records_a_waiting_end_first() {
        let (hub, path) = hub_on_file("presence-end-board");
        let (reg, _cmds, generation) = hub.node_link_opened("n1", Vec::new(), 1).unwrap();
        let other = rusqlite::Connection::open(&path).unwrap();
        other.execute_batch(REFUSE_EVENTS).unwrap();
        assert!(hub.node_link_closed(reg, generation, false, 5, 6));
        let call = agora_protocol::v1::BoardCall {
            from_agent: "a".into(),
            project_id: "p".into(),
            tool: "task_list".into(),
            arguments_json: "{}".into(),
            ..Default::default()
        };
        match hub.board_call("n2", &call, 7).verdict {
            crate::board::Verdict::Invalid(e) => assert_eq!(
                e,
                agora_protocol::Error::new(agora_protocol::ErrorCode::Internal)
                    .with("incident_id", "board.store")
            ),
            _ => panic!("a board call went ahead while a machine's end could not be recorded"),
        }
        assert!(hub.has_unrecorded());
        other.execute_batch("DROP TRIGGER no_events;").unwrap();
        let _ = hub.board_call("n2", &call, 8);
        assert!(!hub.has_unrecorded());
        assert!(!hub.view().0.nodes["n1"].online);
        drop(hub);
        remove_db(&path);
    }

    /// #996: a waiting end the store does not apply (the hub's count of its own facts no longer matches the store's) is not
    /// dropped: the attempt fails, the count is read again, and the next attempt writes it.
    #[test]
    fn a_waiting_end_the_store_does_not_apply_is_kept_and_written_later() {
        let (hub, path) = hub_on_file("presence-end-unapplied");
        let (reg, _cmds, generation) = hub.node_link_opened("n1", Vec::new(), 1).unwrap();
        let other = rusqlite::Connection::open(&path).unwrap();
        other.execute_batch(REFUSE_EVENTS).unwrap();
        assert!(hub.node_link_closed(reg, generation, false, 5, 6));
        other
            .execute_batch("DROP TRIGGER no_events; UPDATE producers SET last_seq = last_seq + 1 WHERE producer = 'hub';")
            .unwrap();
        assert!(hub.record_unrecorded().is_err());
        assert!(hub.has_unrecorded());
        assert!(hub.view().0.nodes["n1"].online);
        hub.record_unrecorded().unwrap();
        assert!(!hub.has_unrecorded());
        assert!(!hub.view().0.nodes["n1"].online);
        assert!(!View::rebuild(hub.store()).unwrap().0.nodes["n1"].online);
        drop(hub);
        remove_db(&path);
    }

    /// #996: a machine that dials again after its last end could not be recorded is recorded down, then up, in that order, so the
    /// late end never hides the new connection. While the store still refuses, the new connection is not kept (#957); once it
    /// works, the next one is recorded online, with the next generation.
    #[test]
    fn a_machine_whose_end_was_not_recorded_comes_back_online_in_order() {
        let (hub, path) = hub_on_file("presence-end-then-open");
        let (reg, _cmds, generation) = hub.node_link_opened("n1", Vec::new(), 1).unwrap();
        let other = rusqlite::Connection::open(&path).unwrap();
        other.execute_batch(REFUSE_EVENTS).unwrap();
        assert!(hub.node_link_closed(reg, generation, true, 5, 6));
        assert!(hub.node_link_opened("n1", Vec::new(), 7).is_err());
        assert!(!hub.router().is_connected("n1"));
        other.execute_batch("DROP TRIGGER no_events;").unwrap();
        let (_reg, _cmds, generation) = hub.node_link_opened("n1", Vec::new(), 8).unwrap();
        assert_eq!(generation, 2);
        assert!(!hub.has_unrecorded());
        assert!(hub.router().is_connected("n1"));
        assert!(hub.view().0.nodes["n1"].online);
        assert!(View::rebuild(hub.store()).unwrap().0.nodes["n1"].online);
        assert_eq!(
            presence(&hub, "n1"),
            [
                (true, NodeLinkReason::Opened as i32, 1),
                (false, -1, 0),
                (false, NodeLinkReason::Silent as i32, 1),
                (true, NodeLinkReason::Opened as i32, 2),
            ]
        );
        drop(hub);
        remove_db(&path);
    }

    /// How a machine's end reaches the log in [`rejoin_asks`].
    #[derive(Clone, Copy, PartialEq, Debug)]
    enum End {
        /// Recorded when the connection ends, and looked at (as `server.rs` does) before the machine dials again.
        Recorded,
        /// Refused when the connection ends; the machine's redial writes it, then its own `up`, in one step.
        Owed,
        /// Refused when the connection ends; a first redial writes it but not its own `up`, and a second redial writes the `up`.
        OwedSplit,
    }

    /// `(node, project)` pairs board membership asks to hand over their local work.
    type Asks = Vec<(String, String)>;

    /// #1036: what board membership asks when `n1` leaves the project `p` it shares with the other `nodes` and dials again (each
    /// node runs an agent in `p`), and what the look after that asks.
    fn rejoin_asks(name: &str, nodes: &[&str], end: End) -> (Asks, Asks) {
        let (hub, path) = hub_on_file(name);
        let sup = support();
        let mut links = Vec::new();
        for n in nodes {
            let mut s = open_session(&hub, &sup, n);
            links.push(hub.node_link_opened(n, Vec::new(), 1).unwrap());
            hub.ingest(&mut s, &online(n, 1)).unwrap();
            hub.ingest(&mut s, &agent(n, 2, "a")).unwrap();
        }
        // The project became shared: every node was asked, and every one answered.
        assert_eq!(hub.board_membership(3).len(), nodes.len());
        assert!(hub.board_membership(3).is_empty());
        for n in nodes {
            hub.board_join_settled(n, "p");
        }
        let other = rusqlite::Connection::open(&path).unwrap();
        if end != End::Recorded {
            other.execute_batch(REFUSE_EVENTS).unwrap();
        }
        let (reg, _cmds, generation) = links.remove(0);
        assert!(hub.node_link_closed(reg, generation, false, 5, 6));
        // What the server does right after a connection's end.
        assert!(hub.board_membership(6).is_empty());
        if end == End::OwedSplit {
            let held: i64 = other
                .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
                .unwrap();
            other
                .execute_batch(&format!(
                    "DROP TRIGGER no_events; CREATE TRIGGER no_events BEFORE INSERT ON events WHEN (SELECT COUNT(*) FROM events) >= {} BEGIN SELECT RAISE(ABORT, 'full'); END;",
                    held + 2
                ))
                .unwrap();
            assert!(hub.node_link_opened("n1", Vec::new(), 7).is_err());
            assert!(!hub.has_unrecorded());
            assert!(!hub.view().0.nodes["n1"].online);
        }
        if end != End::Recorded {
            other.execute_batch("DROP TRIGGER no_events;").unwrap();
        }
        let _back = hub.node_link_opened("n1", Vec::new(), 8).unwrap();
        assert!(!hub.has_unrecorded());
        assert!(hub.view().0.nodes["n1"].online);
        let mut asked = hub.board_membership(8);
        asked.sort();
        let again = hub.board_membership(9);
        drop(hub);
        remove_db(&path);
        (asked, again)
    }

    /// #1036: a machine whose end the store refused, and whose own redial then writes that end and its new `up` together, is asked
    /// to hand over its local board work (ADR-0039 amendment B) exactly as after an end recorded at once, and only once; so are the
    /// other nodes the same look asks. Before, board membership saw the machine online at both looks and asked nobody.
    #[test]
    fn a_machine_whose_end_is_written_by_its_own_redial_is_asked_to_rejoin_the_board() {
        let pairs = |nodes: &[&str]| -> Asks {
            nodes
                .iter()
                .map(|n| ((*n).to_string(), "p".to_string()))
                .collect()
        };
        // Three nodes: the project stays shared while n1 is away, so n1 alone is asked when it is back. Two nodes: it stops being
        // shared, so both are asked.
        for (nodes, expected) in [
            (&["n1", "n2", "n3"][..], pairs(&["n1"])),
            (&["n1", "n2"][..], pairs(&["n1", "n2"])),
        ] {
            for end in [End::Recorded, End::Owed, End::OwedSplit] {
                let name = format!("board-rejoin-{}-{end:?}", nodes.len());
                assert_eq!(
                    rejoin_asks(&name, nodes, end),
                    (expected.clone(), Vec::new()),
                    "{nodes:?} {end:?}"
                );
            }
        }
    }

    /// ADR-0112: `fleet_status` is answered on a solo hub, refused `board.not_allowed` (no parameters) once the hub has people
    /// besides the owner, and answered again when the owner turns it on; it names no project.
    #[test]
    fn fleet_status_is_answered_alone_and_on_a_team_only_with_the_owners_switch() {
        let (hub, sup) = (hub(), support());
        let mut s = open_session(&hub, &sup, "n1");
        hub.ingest(&mut s, &online("n1", 1)).unwrap();
        // Amendment A: n1's link agreed on the view; n2's did not (its policy, or an older node), so it is left out.
        let _n1 = hub
            .node_link_opened(
                "n1",
                vec![agora_protocol::handshake::FLEET_STATUS_V1.into()],
                2,
            )
            .unwrap();
        let mut s2 = open_session(&hub, &sup, "n2");
        hub.ingest(&mut s2, &online("n2", 1)).unwrap();
        let _n2 = hub.node_link_opened("n2", Vec::new(), 3).unwrap();
        let call = agora_protocol::v1::BoardCall {
            from_agent: "a".into(),
            tool: "fleet_status".into(),
            arguments_json: "{}".into(),
            ..Default::default()
        };
        let answered = |hub: &Hub| match hub.board_call("n1", &call, 5).verdict {
            crate::board::Verdict::Done { result, is_error } => {
                let text: serde_json::Value =
                    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
                (text, is_error)
            }
            other => panic!("{other:?}"),
        };
        let (text, is_error) = answered(&hub);
        assert!(!is_error, "{text}");
        assert_eq!(text["machines"][0]["id"], "n1", "{text}");
        assert_eq!(text["machines"].as_array().unwrap().len(), 1, "{text}");
        assert_eq!(text["partial"], false);
        // Once n2 connects again and agrees, it is listed; a later link that does not agree takes it out again.
        let _n2 = hub
            .node_link_opened(
                "n2",
                vec![agora_protocol::handshake::FLEET_STATUS_V1.into()],
                4,
            )
            .unwrap();
        assert_eq!(answered(&hub).0["machines"].as_array().unwrap().len(), 2);
        let _n2 = hub.node_link_opened("n2", Vec::new(), 5).unwrap();
        assert_eq!(answered(&hub).0["machines"].as_array().unwrap().len(), 1);

        crate::users::Users::new(hub.store())
            .add("Eda", crate::users::Role::Member, 2)
            .unwrap();
        let (text, is_error) = answered(&hub);
        assert!(is_error);
        assert_eq!(
            text,
            serde_json::json!({"error": {"code": "board.not_allowed", "params": {}}})
        );

        crate::users::set_fleet_switch(hub.store(), true, 3).unwrap();
        let (text, is_error) = answered(&hub);
        assert!(!is_error, "{text}");
        assert_eq!(text["machines"][0]["id"], "n1");
        crate::users::set_fleet_switch(hub.store(), false, 4).unwrap();
        assert!(answered(&hub).1);

        // It names no project and takes an object.
        for (project, args, field) in [("p", "{}", "project_id"), ("", "[]", "arguments_json")] {
            let bad = agora_protocol::v1::BoardCall {
                project_id: project.into(),
                arguments_json: args.into(),
                ..call.clone()
            };
            match hub.board_call("n1", &bad, 6).verdict {
                crate::board::Verdict::Invalid(e) => {
                    assert_eq!(e.code, agora_protocol::ErrorCode::ProtocolInvalidMessage);
                    assert_eq!(
                        e.params.get("field"),
                        Some(&agora_protocol::ParamValue::Str(field.into()))
                    );
                }
                other => panic!("{other:?}"),
            }
        }
    }
}
