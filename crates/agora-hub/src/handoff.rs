//! Durable delivery of a shared task handoff (ADR-0109).

use std::collections::BTreeMap;
use std::time::Duration;

use agora_protocol::handshake::TASK_HANDOFF_V1;
use agora_protocol::v1::{self, command_ok, command_request, command_response};
use agora_protocol::{Error, ErrorCode};
use prost::Message;

use crate::hub::Hub;
use crate::server::now_ms;
use crate::store::{SqliteStore, StoreError};

pub(crate) const WINDOW_MS: u64 = 24 * 60 * 60 * 1000;
const MAX_COUNT: usize = 1_024;
const MAX_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MAX_SETTLED: usize = 256;
const MAX_IN_FLIGHT: usize = 16;
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, PartialEq, Message)]
pub(crate) struct Pending {
    #[prost(message, required, tag = "1")]
    pub delivery: v1::DeliverTaskHandoff,
    #[prost(string, tag = "2")]
    pub receiver_node: String,
    #[prost(string, tag = "3")]
    pub board_project: String,
    #[prost(uint64, tag = "4")]
    pub task_revision: u64,
    #[prost(bytes = "vec", tag = "5")]
    pub giver_run: Vec<u8>,
    #[prost(uint64, tag = "6")]
    pub giver_started_seq: u64,
    #[prost(bool, tag = "7")]
    pub attempted: bool,
    #[prost(bool, tag = "8")]
    pub acknowledged_while_reattaching: bool,
}

impl Pending {
    pub(crate) fn receiver(&self) -> String {
        crate::projection::qualified(&self.receiver_node, &self.delivery.agent_id)
    }

    pub(crate) fn receiver_run<'a>(
        &self,
        view: &'a crate::projection::View,
    ) -> Option<&'a crate::projection::AgentView> {
        view.agents.get(&self.receiver()).filter(|a| {
            a.project_id == self.board_project
                && a.run_id
                    .is_some_and(|run| run.as_slice() == self.delivery.receiver_run_id)
        })
    }

    pub(crate) fn awaiting_restart_start(&self, view: &crate::projection::View) -> bool {
        self.receiver_run(view)
            .is_some_and(|a| a.awaiting_restart_start)
    }

    fn reserved_bytes(&self) -> usize {
        // The first attempt and an acknowledgement while reattaching may add a tag and value.
        self.encoded_len()
            + 2 * usize::from(!self.attempted)
            + 2 * usize::from(!self.acknowledged_while_reattaching)
    }

    pub(crate) fn id(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(self.delivery.delivery_id.len() * 2);
        for &byte in &self.delivery.delivery_id {
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0xf)]));
        }
        out
    }

    pub(crate) fn key(&self) -> String {
        format!("task-handoff.{}", self.id())
    }

    pub(crate) fn status(
        &self,
        state: v1::TaskHandoffDeliveryState,
        changed_at_ms: u64,
        error_code: &str,
    ) -> v1::event::Body {
        v1::event::Body::TaskHandoffDeliveryChanged(v1::TaskHandoffDeliveryChanged {
            delivery_id: self.id(),
            project_id: self.board_project.clone(),
            task_id: self.delivery.task_id.clone(),
            receiver_agent_id: format!("{}/{}", self.receiver_node, self.delivery.agent_id),
            state: state as i32,
            changed_at_ms,
            error_code: error_code.to_string(),
            acknowledged: false,
        })
    }
}

pub(crate) fn is_settled(status: &v1::TaskHandoffDeliveryChanged) -> bool {
    matches!(
        status.state(),
        v1::TaskHandoffDeliveryState::Delivered | v1::TaskHandoffDeliveryState::Failed
    ) || status.acknowledged
}

pub(crate) type Queue = BTreeMap<Vec<u8>, Pending>;

pub(crate) fn load(store: &SqliteStore) -> Result<Queue, StoreError> {
    let mut queue = Queue::new();
    let mut total = 0;
    for bytes in store.handoff_settings()? {
        let pending = Pending::decode(bytes.as_slice()).map_err(|e| StoreError(e.to_string()))?;
        total += bytes.len().max(pending.reserved_bytes());
        if pending.delivery.delivery_id.len() != 16
            || pending.delivery.receiver_run_id.len() != 16
            || queue
                .insert(pending.delivery.delivery_id.clone(), pending)
                .is_some()
            || queue.len() > MAX_COUNT
            // A queue saved before ADR-0124 did not reserve bytes for deferred acknowledgements.
            || total > MAX_BYTES + 2 * MAX_COUNT
        {
            return Err(StoreError("invalid task handoff queue".into()));
        }
    }
    Ok(queue)
}

pub(crate) fn reserve(queue: &Queue, pending: &Pending) -> Result<(), Error> {
    let limit = |resource, limit| {
        Error::new(ErrorCode::NodeResourceLimit)
            .with("resource", resource)
            .with("limit", limit as i64)
    };
    if queue.len() >= MAX_COUNT {
        return Err(limit("handoff_records", MAX_COUNT));
    }
    let used: usize = queue.values().map(Pending::reserved_bytes).sum();
    if used + pending.reserved_bytes() > MAX_BYTES {
        return Err(limit("handoff_bytes", MAX_BYTES));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Attempt {
    Delivered,
    Rejected(ErrorCode),
    Retry,
}

fn classify(pending: &Pending, response: v1::CommandResponse) -> Attempt {
    match response.result {
        Some(command_response::Result::Ok(v1::CommandOk {
            result: Some(command_ok::Result::TaskHandoffReceipt(receipt)),
        })) if receipt.delivery_id == pending.delivery.delivery_id
            && receipt.receiver_run_id == pending.delivery.receiver_run_id =>
        {
            Attempt::Delivered
        }
        Some(command_response::Result::Err(err)) => match ErrorCode::parse(&err.code) {
            code @ (ErrorCode::ApprovalGrantRequired
            | ErrorCode::NodePolicyDenied
            | ErrorCode::MeshRecipientUnknown
            | ErrorCode::MeshBodyTooLarge
            | ErrorCode::MeshTooManyRefs
            | ErrorCode::ProtocolInvalidMessage
            | ErrorCode::ProtocolCapabilityMissing) => Attempt::Rejected(code),
            ErrorCode::NodeResourceLimit
                if err.params.get("resource").is_some_and(|p| {
                    matches!(&p.value, Some(v1::param_value::Value::Str(s)) if
                        matches!(s.as_str(), "handoff_records" | "handoff_bytes"))
                }) =>
            {
                Attempt::Rejected(ErrorCode::NodeResourceLimit)
            }
            _ => Attempt::Retry,
        },
        _ => Attempt::Retry,
    }
}

pub(crate) async fn send_on(router: &crate::router::Router, pending: &Pending) -> Attempt {
    let remaining = pending.delivery.valid_until_ms.saturating_sub(now_ms());
    if remaining == 0 {
        return Attempt::Retry;
    }
    let request = v1::CommandRequest {
        command: Some(command_request::Command::DeliverTaskHandoff(
            pending.delivery.clone(),
        )),
        ..Default::default()
    };
    let Ok(reply) = router.send(&pending.receiver_node, request) else {
        return Attempt::Retry;
    };
    match tokio::time::timeout(ATTEMPT_TIMEOUT.min(Duration::from_millis(remaining)), reply).await {
        Ok(Ok(response)) => classify(pending, response),
        _ => Attempt::Retry,
    }
}

/// One worker per listening hub. Attempts to different machines run concurrently, with one at a time per machine.
pub(crate) async fn run(hub: &Hub) {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut retry = BTreeMap::<Vec<u8>, (u64, u64)>::new();
    let mut settling = BTreeMap::new();
    // ADR-0131: the outcome rows of ended runs are swept once a minute (this worker is the hub's one periodic task).
    let mut swept_at = 0u64;
    loop {
        tick.tick().await;
        let now = now_ms();
        if now.saturating_sub(swept_at) >= 60_000 {
            swept_at = now;
            if let Err(error) = hub.prune_pickup_outcomes() {
                crate::log_line!("task pickup sweep: {error}");
            }
        }
        // Keep answers while their transactions are unavailable, without retrying or expiring those deliveries.
        settling.retain(|_, (pending, outcome)| {
            !matches!(hub.settle_handoff(pending, outcome, now), Ok(true))
        });
        if let Err(error) = hub.expire_handoffs(now, |id| settling.contains_key(id)) {
            crate::log_line!("task handoff expiry: {error}");
        }
        let mut nodes = std::collections::BTreeSet::new();
        let batch = hub.pending_handoffs(
            now,
            |p| {
                !settling.contains_key(&p.delivery.delivery_id)
                    && retry
                        .get(&p.delivery.delivery_id)
                        .is_none_or(|(at, _)| now >= *at)
                    && hub
                        .router()
                        .has_capability(&p.receiver_node, TASK_HANDOFF_V1)
                    && nodes.insert(p.receiver_node.clone())
            },
            MAX_IN_FLIGHT,
        );
        let mut sends = tokio::task::JoinSet::new();
        for pending in batch {
            let Ok(may_have_arrived) = hub.begin_handoff_attempt(&pending, now_ms()) else {
                continue;
            };
            let router = hub.router().clone();
            sends.spawn(async move {
                let outcome = match send_on(&router, &pending).await {
                    Attempt::Rejected(_) if may_have_arrived => Attempt::Retry,
                    outcome => outcome,
                };
                (pending, outcome)
            });
        }
        while let Some(result) = sends.join_next().await {
            let Ok((pending, outcome)) = result else {
                continue;
            };
            let id = pending.delivery.delivery_id.clone();
            if outcome != Attempt::Retry {
                retry.remove(&id);
                let result = hub.settle_handoff(&pending, &outcome, now_ms());
                if let Err(error) = &result {
                    crate::log_line!("task handoff settlement: {error}");
                }
                if !matches!(result, Ok(true)) {
                    settling.insert(id, (pending, outcome));
                }
            } else {
                let entry = retry.entry(id).or_insert((0, 500));
                entry.1 = (entry.1 * 2).min(60_000);
                entry.0 = now_ms().saturating_add(entry.1);
            }
        }
        retry.retain(|id, _| hub.handoff_is_pending(id, now));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_attempt_marker_is_reserved_before_a_delivery_fills_the_queue() {
        let mut pending = Pending::default();
        pending.delivery.summary = "x".repeat(MAX_BYTES);
        let excess = pending.encoded_len() - MAX_BYTES;
        pending.delivery.summary.truncate(MAX_BYTES - excess);
        assert_eq!(pending.encoded_len(), MAX_BYTES);
        assert_eq!(
            reserve(&Queue::new(), &pending).unwrap_err().code,
            ErrorCode::NodeResourceLimit,
        );
        pending.delivery.summary.truncate(MAX_BYTES - excess - 4);
        assert!(reserve(&Queue::new(), &pending).is_ok());
        pending.attempted = true;
        pending.acknowledged_while_reattaching = true;
        assert_eq!(pending.encoded_len(), MAX_BYTES);
        assert!(reserve(&Queue::new(), &pending).is_ok());
    }
}
