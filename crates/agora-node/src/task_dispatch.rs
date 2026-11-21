//! ADR-0095 decision 5 on the node: a task's prompt is never typed over a running turn. A structured agent queues it like any prompt
//! (ADR-0090); a terminal agent has no queue of its own, so the node holds at most one dispatch prompt per terminal agent here and types
//! it once the agent is between turns again. Plain data and a clock; the typing itself is [`type_prompt`].

use std::collections::HashMap;
use std::io;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use agora_core::{AgentId, AgentState};
use agora_protocol::{Error, ErrorCode};

/// How long after a prompt was typed into a terminal agent its detected state is not trusted to say "between turns": detection reads
/// the screen and may not have seen the new turn begin yet.
pub const SETTLE: Duration = Duration::from_secs(5);

/// How many dispatch prompts the node holds for one terminal agent.
pub const MAX_HELD: usize = 1;

/// A dispatch prompt waiting for its terminal agent to be between turns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub prompt: String,
    pub task_id: String,
    /// The task is on the hub's shared board, which cannot give it back.
    pub shared: bool,
}

/// Whether an agent in `state` is between turns: idle, finished and not yet seen, or not known (a terminal agent whose screen says
/// nothing is treated as ready, as a typed prompt is).
pub fn between_turns(state: AgentState) -> bool {
    matches!(
        state,
        AgentState::Idle | AgentState::Done | AgentState::Unknown
    )
}

/// Lowercase hex SHA-256 of the prompt's UTF-8 bytes (`TaskDispatched.prompt_sha256`).
pub fn prompt_sha256(prompt: &str) -> String {
    let d = ring::digest::digest(&ring::digest::SHA256, prompt.as_bytes());
    crate::device_trust::hex(d.as_ref())
}

/// Types `text` into a terminal agent the way a window's prompt is typed: the text, a short pause, then Enter. Two writes: a terminal
/// program that reads both in one go takes them for a paste and keeps the Enter as a newline in its input box. Blocks for the pause.
pub fn type_prompt(control: &agora_platform::PtyControl, text: &str) -> io::Result<()> {
    control.write_input(text.as_bytes())?;
    std::thread::sleep(Duration::from_millis(150));
    control.write_input(b"\r")
}

#[derive(Default)]
struct State {
    held: HashMap<AgentId, Held>,
    typed: HashMap<AgentId, Instant>,
}

/// The node's held dispatch prompts, one per terminal agent at most.
#[derive(Default)]
pub struct HeldDispatches(Mutex<State>);

impl HeldDispatches {
    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether a prompt for `agent` (in `state`) may be typed now: nothing is held for it, it is between turns, and no prompt was typed
    /// into it in the last [`SETTLE`].
    pub fn ready(&self, agent: &AgentId, state: AgentState, now: Instant) -> bool {
        let st = self.st();
        !st.held.contains_key(agent)
            && between_turns(state)
            && st
                .typed
                .get(agent)
                .is_none_or(|t| now.saturating_duration_since(*t) >= SETTLE)
    }

    /// Whether a prompt is held for `agent` (a new dispatch to it is refused before anything is claimed).
    pub fn is_held(&self, agent: &AgentId) -> bool {
        self.st().held.contains_key(agent)
    }

    /// Notes that a prompt was typed into `agent` at `now`.
    pub fn typed(&self, agent: &AgentId, now: Instant) {
        self.st().typed.insert(agent.clone(), now);
    }

    /// Holds `held` for `agent` until it is between turns; one at most (`agent.queue_full`, `limit` 1).
    pub fn hold(&self, agent: &AgentId, held: Held) -> Result<(), Error> {
        let mut st = self.st();
        if st.held.contains_key(agent) {
            return Err(queue_full());
        }
        st.held.insert(agent.clone(), held);
        Ok(())
    }

    /// The held prompts of the terminal agents in `agents` (id and detected state) that are now between turns and settled, taken out
    /// and marked typed at `now`. A held prompt whose agent is not listed stays (see [`Self::take`] for an agent that ended).
    pub fn due<'a>(
        &self,
        agents: impl IntoIterator<Item = (&'a AgentId, AgentState)>,
        now: Instant,
    ) -> Vec<(AgentId, Held)> {
        let mut st = self.st();
        st.typed
            .retain(|_, t| now.saturating_duration_since(*t) < SETTLE);
        if st.held.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (agent, state) in agents {
            if between_turns(state) && !st.typed.contains_key(agent) {
                if let Some(h) = st.held.remove(agent) {
                    st.typed.insert(agent.clone(), now);
                    out.push((agent.clone(), h));
                }
            }
        }
        out
    }

    /// Takes the held prompt of an agent that ended or was stopped (it is never typed), and forgets when it was last typed into.
    pub fn take(&self, agent: &AgentId) -> Option<Held> {
        let mut st = self.st();
        st.typed.remove(agent);
        st.held.remove(agent)
    }
}

fn queue_full() -> Error {
    Error::new(ErrorCode::AgentQueueFull).with("limit", MAX_HELD as i64)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn held(task: &str) -> Held {
        Held {
            prompt: format!("prompt {task}"),
            task_id: task.into(),
            shared: false,
        }
    }

    #[test]
    fn a_busy_terminal_agent_gets_its_prompt_only_once_it_is_between_turns() {
        let h = HeldDispatches::default();
        let a = AgentId::new("a");
        let t0 = Instant::now();
        assert!(!h.ready(&a, AgentState::Working, t0));
        assert!(!h.ready(&a, AgentState::Blocked, t0));
        h.hold(&a, held("t1")).unwrap();
        // One held at most; a second is refused with the queue's code.
        let e = h.hold(&a, held("t2")).unwrap_err();
        assert_eq!(e.code, ErrorCode::AgentQueueFull);
        assert_eq!(
            e.params.get("limit"),
            Some(&agora_protocol::ParamValue::Int(1))
        );
        // While something is held, nothing is typed directly, even when the agent looks idle.
        assert!(!h.ready(&a, AgentState::Idle, t0));
        // Still working, or waiting for an answer: nothing is due.
        assert!(h.due([(&a, AgentState::Working)], t0).is_empty());
        assert!(h.due([(&a, AgentState::Blocked)], t0).is_empty());
        // Idle: the held prompt comes out once.
        let due = h.due([(&a, AgentState::Idle)], t0);
        assert_eq!(due, vec![(a.clone(), held("t1"))]);
        assert!(h.due([(&a, AgentState::Idle)], t0).is_empty());
        // Right after it was typed the screen may still look idle: a new dispatch is held, not typed over the new turn.
        assert!(!h.ready(&a, AgentState::Idle, t0 + Duration::from_secs(1)));
        h.hold(&a, held("t3")).unwrap();
        assert!(h
            .due([(&a, AgentState::Idle)], t0 + Duration::from_secs(1))
            .is_empty());
        assert_eq!(h.due([(&a, AgentState::Idle)], t0 + SETTLE).len(), 1);
    }

    #[test]
    fn an_ended_agents_held_prompt_is_taken_and_never_typed() {
        let h = HeldDispatches::default();
        let (a, b) = (AgentId::new("a"), AgentId::new("b"));
        let t0 = Instant::now();
        h.hold(&a, held("t1")).unwrap();
        h.hold(&b, held("t2")).unwrap();
        assert_eq!(h.take(&a), Some(held("t1")));
        assert_eq!(h.take(&a), None);
        // An agent not listed (not a running terminal agent now) keeps its prompt until it is listed or taken.
        assert!(h.due([], t0).is_empty());
        assert!(h.is_held(&b));
        assert!(h.ready(&a, AgentState::Unknown, t0));
        assert!(h.ready(&a, AgentState::Done, t0));
    }

    #[test]
    fn the_prompt_hash_is_lowercase_hex_sha256() {
        assert_eq!(
            prompt_sha256("test"),
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        );
    }
}
