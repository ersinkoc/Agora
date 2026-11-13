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

use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Mutex, MutexGuard};

use agora_protocol::v1::event::Body;
use agora_protocol::v1::{Event, NodeOffline};
use prost::Message;

use crate::ca::RESERVED_HUB_ID;
use crate::projection::View;
use crate::session::{Ingest, NodeSession, SessionError};
use crate::store::{Append, EventStore, SqliteStore, StoreError};

/// Events a subscriber may have queued before it is considered too slow and dropped.
pub const SUBSCRIBER_BUFFER: usize = 1024;
const PAGE: usize = 1000;

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
}

pub struct Hub {
    boards: crate::board::Boards,
    prices: std::sync::RwLock<(std::sync::Arc<crate::usage::Catalog>, u64)>,
    store: SqliteStore,
    router: std::sync::Arc<crate::router::Router>,
    inner: Mutex<Inner>,
}

fn decode(e: &crate::store::StoredEvent) -> Option<Delivered> {
    Event::decode(e.payload.as_slice())
        .ok()
        .map(|event| Delivered { pos: e.pos, event })
}

impl Hub {
    /// Opens the hub on a store, rebuilding the view from the log.
    pub fn open(store: SqliteStore) -> Result<Self, StoreError> {
        let (view, pos) = View::rebuild(&store)?;
        let hub_seq = store.acked(RESERVED_HUB_ID)?;
        Ok(Self {
            boards: crate::board::Boards::new(),
            prices: std::sync::RwLock::new((crate::usage::Catalog::bundled(), 0)),
            store,
            router: crate::router::Router::new(),
            inner: Mutex::new(Inner {
                view,
                pos,
                hub_seq,
                subs: Vec::new(),
            }),
        })
    }

    /// Replaces the price table (overrides already applied) and says when it was refreshed (ms since the epoch; 0 for the
    /// table bundled with the release).
    pub fn set_prices(&self, catalog: crate::usage::Catalog, at_ms: u64) {
        if let Ok(mut p) = self.prices.write() {
            *p = (std::sync::Arc::new(catalog), at_ms);
        }
    }

    /// Tokens and estimated cost per agent (ADR-0033). An agent whose model has no price has no estimate.
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
                let micro_usd = catalog.cost(&a.model, &usage, None).map(Cost::micro_usd);
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

    /// Where connected nodes wait for commands (ADR-0029).
    pub fn router(&self) -> &std::sync::Arc<crate::router::Router> {
        &self.router
    }

    pub fn store(&self) -> &SqliteStore {
        &self.store
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock cannot leave the view half-applied (apply is the last step), so keep going.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether any node by this id has ever been seen (online or not).
    pub fn node_known(&self, node: &str) -> bool {
        self.lock().view.nodes.contains_key(node)
    }

    /// The current view and the log position it reflects.
    pub fn view(&self) -> (View, u64) {
        let g = self.lock();
        (g.view.clone(), g.pos)
    }

    /// Passes one payload from a node's session through the store, the view and the subscribers.
    pub fn ingest(
        &self,
        session: &mut NodeSession<'_, SqliteStore>,
        payload: &[u8],
    ) -> Result<Ingest, SessionError> {
        let mut g = self.lock();
        let outcome = session.event(payload)?;
        if let Ingest::Applied { pos, .. } | Ingest::Resynced { pos, .. } = outcome {
            // The session already decoded it successfully, so this cannot fail; stay defensive anyway.
            if let Ok(event) = Event::decode(payload) {
                let restarted = match &event.body {
                    Some(Body::NodeOnline(n)) => Some((n.node_id.clone(), event.at_ms)),
                    _ => None,
                };
                Self::publish(&mut g, Delivered { pos, event });
                // A node announces itself only when its process starts (a resync re-states it only for a start the hub missed,
                // ADR-0055): the agents it had are gone, so their tasks and leases go back.
                if let Some((node, at_ms)) = restarted {
                    for (project, events) in self.boards.node_restarted(&node, at_ms) {
                        if let Some(bytes) = self.boards.snapshot(&project, at_ms) {
                            let _ = self.store.set_setting(&format!("board.{project}"), &bytes);
                        }
                        for body in events {
                            let _ = self.emit(&mut g, body, at_ms);
                        }
                    }
                }
            }
        }
        Ok(outcome)
    }

    /// ADR-0053: the hub stops listing a closed agent. `agent` is the hub-wide key. Refused while it runs; its earlier events stay in
    /// the store, and its tokens and cost still count.
    pub fn forget_agent(&self, agent: &str, now_ms: u64) -> Result<(), agora_protocol::Error> {
        use agora_protocol::{Error, ErrorCode};
        let mut g = self.lock();
        match g.view.agents.get(agent) {
            None => return Err(Error::new(ErrorCode::AgentNotFound).with("agent_id", agent)),
            Some(a) if a.running => {
                return Err(Error::new(ErrorCode::AgentRunning).with("agent_id", agent))
            }
            Some(a) if a.forgotten => return Ok(()),
            Some(_) => {}
        }
        self.emit(
            &mut g,
            Body::AgentForgotten(agora_protocol::v1::AgentForgotten {
                agent_id: agent.to_string(),
            }),
            now_ms,
        )
        .map_err(|_| Error::new(ErrorCode::Internal))
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

    /// Records an event the hub itself produced (as the reserved hub producer).
    fn emit(&self, g: &mut Inner, body: Body, now_ms: u64) -> Result<(), StoreError> {
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
        }
        Ok(())
    }

    /// ADR-0039 amendment B: the `(node, project)` pairs that must hand over their local work now (see `Boards::membership`).
    pub fn board_membership(&self, now_ms: u64) -> Vec<(String, String)> {
        let g = self.lock();
        self.boards.membership(&g.view, now_ms)
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
        let key = self.restore_board(project, now_ms);
        let (events, notices) = self.boards.joined(&g.view, node, project, work, now_ms);
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
        let mut g = self.lock();
        let key = self.restore_board(&call.project_id, now_ms);
        let mut a = self.boards.call(
            &g.view,
            node,
            &call.from_agent,
            &call.project_id,
            &call.tool,
            &call.arguments_json,
            now_ms,
        );
        if !a.events.is_empty() {
            // Tasks changed: keep the board in the store (best effort; the call already succeeded).
            if let Some(bytes) = self.boards.snapshot(&call.project_id, now_ms) {
                let _ = self.store.set_setting(&key, &bytes);
            }
        }
        for body in std::mem::take(&mut a.events) {
            let _ = self.emit(&mut g, body, now_ms);
        }
        a
    }

    /// Restores an uninitialized board and returns its storage key. Callers hold the hub lock so calls and joins cannot
    /// reload over one another's newer in-memory work.
    fn restore_board(&self, project: &str, now_ms: u64) -> String {
        let key = format!("board.{project}");
        if !self.boards.has(project) {
            if let Ok(Some(bytes)) = self.store.setting(&key) {
                self.boards.load(project, &bytes, now_ms);
            }
        }
        key
    }

    fn publish(g: &mut Inner, d: Delivered) {
        g.view.apply(&d.event);
        g.pos = d.pos;
        // `try_send`: a full queue means the subscriber is too slow; drop it rather than stall every other node.
        g.subs.retain(|s| match s.try_send(d.clone()) {
            Ok(()) => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => false,
        });
    }

    /// Subscribes from `after` (the last position the subscriber saw), or from the start with `None`. If `after`
    /// is ahead of the log (the hub was restored from an older backup) the subscriber gets a snapshot instead.
    pub fn subscribe(&self, after: Option<u64>) -> Result<Subscription, StoreError> {
        let mut g = self.lock();
        let (tx, rx) = sync_channel(SUBSCRIBER_BUFFER);
        let sub = match after {
            Some(n) if n <= g.pos => {
                let mut backlog = Vec::new();
                let mut at = n;
                loop {
                    let page = self.store.events_after(at, PAGE)?;
                    let Some(last) = page.last() else { break };
                    at = last.pos;
                    backlog.extend(page.iter().filter_map(decode));
                }
                Subscription {
                    snapshot: None,
                    backlog,
                    live: rx,
                }
            }
            _ => Subscription {
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_protocol::handshake::Support;
    use agora_protocol::v1::{AgentStarted, Hello, NodeOnline, Resume, Role};

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
    ) -> NodeSession<'a, SqliteStore> {
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
                agent_id: id.into(),
                project_id: "p".into(),
                node_id: node.into(),
                worktree_id: "w".into(),
                kind: "shell".into(),
                level: 2,
                parent_agent_id: String::new(),
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
}
