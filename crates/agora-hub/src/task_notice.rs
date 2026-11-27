//! ADR-0128 parent notices. A notice follows a committed human acceptance and has no task-state compensation.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use agora_mesh::broker::ReviewParent;
use agora_protocol::v1::{self, command_ok, command_request, command_response};
use agora_protocol::ErrorCode;
use prost::Message;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::hub::Hub;
use crate::projection::{qualified, View};
use crate::server::now_ms;
use crate::store::{EventStore, SqliteStore, StoreError};

const WINDOW_MS: u64 = 24 * 60 * 60 * 1000;
const MAX_COUNT: usize = 1024;
const MAX_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const QUEUE_PREFIX: &str = "task-acceptance-notice.";
const ENABLE_PREFIX: &str = "notify-task-parent.";
const VERIFIED_PREFIX: &str = "verified-task-parent.";

#[derive(Clone, PartialEq, Message)]
pub(crate) struct Pending {
    #[prost(string, tag = "1")]
    pub node: String,
    #[prost(message, required, tag = "2")]
    pub notice: v1::DeliverTaskAcceptanceNotice,
}

pub(crate) type Queue = BTreeMap<Vec<u8>, Pending>;

fn hash(parts: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part);
    }
    digest.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn verified_key(node: &str, agent: &str) -> String {
    format!(
        "{VERIFIED_PREFIX}{}",
        hex(&hash(&[node.as_bytes(), agent.as_bytes()]))
    )
}

pub(crate) fn parent_json(parent: &ReviewParent) -> Value {
    json!({"node": parent.node_id, "agent": parent.agent_id, "run": parent.run_id, "project": parent.project_id})
}

pub(crate) fn parse_parent(value: &Value) -> Option<ReviewParent> {
    Some(ReviewParent {
        node_id: value["node"].as_str()?.to_string(),
        agent_id: value["agent"].as_str()?.to_string(),
        run_id: serde_json::from_value(value["run"].clone()).ok()?,
        project_id: value["project"].as_str()?.to_string(),
    })
}

pub(crate) fn remember(
    store: &SqliteStore,
    verified: &crate::helpers::VerifiedParent,
) -> Result<(), StoreError> {
    store.set_setting(
        &verified_key(&verified.helper_node, &verified.helper_agent),
        json!({"helper_run": verified.helper_run, "parent": parent_json(&verified.parent)})
            .to_string()
            .as_bytes(),
    )
}

pub(crate) fn verified_parent(
    store: &SqliteStore,
    node: &str,
    agent: &str,
    run: [u8; 16],
) -> Result<Option<ReviewParent>, StoreError> {
    let value = store.setting(&verified_key(node, agent))?;
    Ok(value.and_then(|bytes| {
        let row: Value = serde_json::from_slice(&bytes).ok()?;
        let saved: [u8; 16] = serde_json::from_value(row["helper_run"].clone()).ok()?;
        if saved != run {
            return None;
        }
        parse_parent(&row["parent"])
    }))
}

pub(crate) fn enable(store: &SqliteStore, project: &str) -> Result<(), StoreError> {
    store.set_setting(&format!("{ENABLE_PREFIX}{project}"), b"on")
}

pub(crate) fn enabled(store: &SqliteStore, project: &str) -> Result<bool, StoreError> {
    Ok(store
        .setting(&format!("{ENABLE_PREFIX}{project}"))?
        .as_deref()
        == Some(b"on"))
}

pub(crate) fn load(store: &SqliteStore) -> Result<Queue, StoreError> {
    let mut queue = Queue::new();
    for bytes in store.settings_with_prefix(QUEUE_PREFIX)? {
        let pending = Pending::decode(bytes.as_slice()).map_err(|e| StoreError(e.to_string()))?;
        if !room_for(&queue, &pending)
            || pending.notice.delivery_id.len() != 16
            || pending.notice.receiver_run_id.len() != 16
            || queue
                .insert(pending.notice.delivery_id.clone(), pending)
                .is_some()
        {
            return Err(StoreError("invalid task acceptance notice queue".into()));
        }
    }
    Ok(queue)
}

pub(crate) fn room_for(queue: &Queue, pending: &Pending) -> bool {
    queue.len() < MAX_COUNT
        && queue.values().map(Message::encoded_len).sum::<usize>() + pending.encoded_len()
            <= MAX_BYTES
}

impl Pending {
    pub(crate) fn new(parent: &ReviewParent, task: &str, revision: u64, now: u64) -> Self {
        let id = hash(&[
            b"task-acceptance-notice-v1",
            parent.project_id.as_bytes(),
            task.as_bytes(),
            &revision.to_le_bytes(),
        ]);
        Self {
            node: parent.node_id.clone(),
            notice: v1::DeliverTaskAcceptanceNotice {
                delivery_id: id[..16].to_vec(),
                agent_id: parent.agent_id.clone(),
                receiver_run_id: parent.run_id.to_vec(),
                project_id: parent.project_id.clone(),
                task_id: task.into(),
                report_revision: revision,
                accepted_at_ms: now,
                valid_until_ms: now.saturating_add(WINDOW_MS),
            },
        }
    }

    pub(crate) fn key(&self) -> String {
        format!("{QUEUE_PREFIX}{}", hex(&self.notice.delivery_id))
    }

    pub(crate) fn receiver(&self, view: &View) -> Option<bool> {
        view.agents
            .get(&qualified(&self.node, &self.notice.agent_id))
            .map(|agent| {
                !agent.forgotten
                    && agent.project_id == self.notice.project_id
                    && agent
                        .run_id
                        .is_some_and(|run| run.as_slice() == self.notice.receiver_run_id)
                    && (agent.running || agent.awaiting_restart_start)
            })
    }

    pub(crate) fn ready(&self, view: &View) -> bool {
        self.receiver(view) == Some(true)
            && view
                .agents
                .get(&qualified(&self.node, &self.notice.agent_id))
                .is_some_and(|agent| agent.running && !agent.awaiting_restart_start)
    }
}

fn settled(p: &Pending, response: v1::CommandResponse) -> bool {
    match response.result {
        Some(command_response::Result::Ok(v1::CommandOk {
            result: Some(command_ok::Result::TaskAcceptanceNoticeReceipt(receipt)),
        })) => {
            receipt.delivery_id == p.notice.delivery_id
                && receipt.receiver_run_id == p.notice.receiver_run_id
        }
        Some(command_response::Result::Err(err)) => matches!(
            ErrorCode::parse(&err.code),
            ErrorCode::NodePolicyDenied | ErrorCode::ApprovalGrantRequired
        ),
        None | Some(command_response::Result::Ok(_)) => false,
    }
}

async fn send_on(
    router: std::sync::Arc<crate::router::Router>,
    pending: Pending,
) -> (Pending, bool) {
    let remaining = pending.notice.valid_until_ms.saturating_sub(now_ms());
    if remaining == 0 {
        return (pending, true);
    }
    let request = v1::CommandRequest {
        command: Some(command_request::Command::DeliverTaskAcceptanceNotice(
            pending.notice.clone(),
        )),
        ..Default::default()
    };
    let Ok(reply) = router.send(&pending.node, request) else {
        return (pending, false);
    };
    let timeout = Duration::from_millis(remaining.min(5_000));
    let done = match tokio::time::timeout(timeout, reply).await {
        Ok(Ok(response)) => settled(&pending, response),
        _ => false,
    };
    (pending, done)
}

pub(crate) async fn run(hub: &Hub) {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut retry = BTreeMap::<Vec<u8>, (u64, u64)>::new();
    let mut receipts = BTreeSet::<Vec<u8>>::new();
    loop {
        tick.tick().await;
        let now = now_ms();
        let pending = match hub.task_notice_batch(now, &receipts, &retry) {
            Ok(pending) => pending,
            Err(error) => {
                crate::log_line!("task acceptance notice housekeeping: {error}");
                continue;
            }
        };
        let mut sends = tokio::task::JoinSet::new();
        for notice in pending {
            sends.spawn(send_on(hub.router().clone(), notice));
        }
        while let Some(result) = sends.join_next().await {
            let Ok((pending, done)) = result else {
                continue;
            };
            let id = pending.notice.delivery_id;
            if done {
                receipts.insert(id);
            } else {
                let entry = retry.entry(id).or_insert((0, 500));
                entry.1 = (entry.1 * 2).min(60_000);
                entry.0 = now_ms().saturating_add(entry.1);
            }
        }
        receipts.retain(|id| hub.task_notice_pending(id));
        retry.retain(|id, _| hub.task_notice_pending(id));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn parent() -> ReviewParent {
        ReviewParent {
            node_id: "node".into(),
            agent_id: "agent".into(),
            run_id: [1; 16],
            project_id: "project".into(),
        }
    }

    #[test]
    fn notice_identity_is_stable_and_receipts_must_match_the_run() {
        let pending = Pending::new(&parent(), "t-1", 2, 10);
        assert_eq!(
            pending.notice.delivery_id,
            Pending::new(&parent(), "t-1", 2, 11).notice.delivery_id
        );
        assert_ne!(
            pending.notice.delivery_id,
            Pending::new(&parent(), "t-1", 3, 10).notice.delivery_id
        );
        let ack = |run: Vec<u8>| v1::CommandResponse {
            result: Some(command_response::Result::Ok(v1::CommandOk {
                result: Some(command_ok::Result::TaskAcceptanceNoticeReceipt(
                    v1::TaskAcceptanceNoticeReceipt {
                        delivery_id: pending.notice.delivery_id.clone(),
                        receiver_run_id: run,
                    },
                )),
            })),
            ..Default::default()
        };
        assert!(!settled(&pending, ack(vec![2; 16])));
        assert!(settled(&pending, ack(vec![1; 16])));
    }

    #[test]
    fn notice_capacity_does_not_use_handoff_reservations() {
        let parent = parent();
        let queue: Queue = (0..MAX_COUNT)
            .map(|n| {
                let pending = Pending::new(&parent, &format!("t-{n}"), 1, 1);
                (pending.notice.delivery_id.clone(), pending)
            })
            .collect();
        assert_eq!(queue.len(), MAX_COUNT);
        assert!(!room_for(&queue, &Pending::new(&parent, "t-2000", 1, 1)));
        assert!(crate::handoff::reserve(
            &Default::default(),
            &crate::handoff::Pending {
                delivery: v1::DeliverTaskHandoff::default(),
                receiver_node: "node".into(),
                board_project: "project".into(),
                task_revision: 1,
                giver_run: vec![1; 16],
                giver_started_seq: 0,
                attempted: false,
                acknowledged_while_reattaching: false,
            }
        )
        .is_ok());
    }
}
