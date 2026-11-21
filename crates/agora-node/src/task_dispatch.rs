//! ADR-0095 decision 5 on the node: a task's prompt is never typed over a running turn (as far as the node can tell; see
//! [`between_turns`]). A structured agent queues it like any prompt (ADR-0090); a terminal agent has no queue of its own, so the node
//! holds at most one dispatch prompt per terminal agent here and pastes it once the agent is between turns again. Plain data and a
//! clock; the pasting itself is [`paste_prompt`].
//!
//! Every dispatch that has not reached its agent yet is remembered here with its task, so a prompt that never arrives (the agent
//! stopped, ended or crashed, the queued send failed or was dropped, the paste failed) gives its task back on the node's own broker.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use agora_core::{AgentId, AgentState};
use agora_protocol::{Error, ErrorCode};

/// How long after a prompt was pasted into a terminal agent its detected state is not trusted to say "between turns": detection reads
/// the screen and may not have seen the new turn begin yet.
pub const SETTLE: Duration = Duration::from_secs(5);

/// How many dispatch prompts the node holds for one terminal agent.
pub const MAX_HELD: usize = 1;

/// The task a dispatch claimed, to give back if its prompt never reaches the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Given {
    pub agent: AgentId,
    pub task_id: String,
    /// The task is on the hub's shared board, which cannot give it back.
    pub shared: bool,
}

/// A dispatch prompt waiting for its terminal agent to be between turns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub prompt: String,
    pub given: Given,
}

/// What [`Dispatches::begin`] decided for a terminal agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// Between turns and settled: paste now (the paste is already counted, so a second dispatch at the same moment is held).
    PasteNow,
    /// In a turn: the agent's one hold slot is reserved; fill it with [`Dispatches::fill`] or let it go with [`Dispatches::cancel`].
    Hold,
}

/// One agent as the supervision loop sees it, for [`Dispatches::tick`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub id: AgentId,
    pub state: AgentState,
    pub running: bool,
    pub terminal: bool,
}

/// Whether an agent in `state` is between turns: idle, finished and not yet seen, or not known. Unknown counts as between turns: a kind
/// whose screen the node cannot read would otherwise never get a held prompt; such a prompt can land while the program works, inside
/// its paste brackets (the program decides what a paste during a turn does).
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

/// The bytes that paste `prompt` into a terminal program that turned on bracketed paste (DECSET 2004): its lines arrive as one paste,
/// so a line break does not submit part of it. The prompt holds no ESC (`agora_mesh::dispatch::build_prompt` removes every control
/// character but line break and tab, C1 included), so nothing in it can end the paste; any ESC is removed here as well.
pub fn paste_bytes(prompt: &str) -> Vec<u8> {
    let body: String = prompt.chars().filter(|&c| c != '\u{1b}').collect();
    format!("\x1b[200~{body}\x1b[201~").into_bytes()
}

/// Types `text` into a terminal agent the way a window's prompt is typed: the text, a short pause, then Enter. Two writes: a terminal
/// program that reads both in one go takes them for a paste and keeps the Enter as a newline in its input box. Blocks for the pause.
pub fn type_prompt(control: &agora_platform::PtyControl, text: &str) -> io::Result<()> {
    control.write_input(text.as_bytes())?;
    std::thread::sleep(Duration::from_millis(150));
    control.write_input(b"\r")
}

/// ADR-0095: pastes a dispatch prompt ([`paste_bytes`]), then, after a short pause, Enter. Blocks for the pause.
pub fn paste_prompt(control: &agora_platform::PtyControl, prompt: &str) -> io::Result<()> {
    control.write_input(&paste_bytes(prompt))?;
    std::thread::sleep(Duration::from_millis(150));
    control.write_input(b"\r")
}

#[derive(Default)]
struct State {
    /// A terminal agent's hold slot: `None` while a dispatch reserved it and is claiming its task.
    held: HashMap<AgentId, Option<Held>>,
    /// When a prompt was last pasted into each terminal agent (kept for [`SETTLE`]).
    pasted: HashMap<AgentId, Instant>,
    /// Dispatch prompts queued behind a structured agent's turn, by the tag the queue gave them.
    queued: HashMap<(AgentId, u64), Given>,
}

/// The node's dispatches that have not reached their agent yet.
#[derive(Default)]
pub struct Dispatches(Mutex<State>);

fn queue_full() -> Error {
    Error::new(ErrorCode::AgentQueueFull).with("limit", MAX_HELD as i64)
}

impl Dispatches {
    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Decides, in one step, what a dispatch to terminal agent `agent` (in `state`) does, before its task is claimed: paste now when the
    /// agent is between turns, nothing is held for it and nothing was pasted in the last [`SETTLE`] (the paste is counted at once);
    /// otherwise reserve its one hold slot. A slot already reserved or filled is `agent.queue_full` (`limit` 1).
    pub fn begin(&self, agent: &AgentId, state: AgentState, now: Instant) -> Result<Slot, Error> {
        let mut st = self.st();
        if st.held.contains_key(agent) {
            return Err(queue_full());
        }
        let settled = st
            .pasted
            .get(agent)
            .is_none_or(|t| now.saturating_duration_since(*t) >= SETTLE);
        if between_turns(state) && settled {
            st.pasted.insert(agent.clone(), now);
            Ok(Slot::PasteNow)
        } else {
            st.held.insert(agent.clone(), None);
            Ok(Slot::Hold)
        }
    }

    /// Fills the slot [`Self::begin`] reserved for `held.given.agent`; false when it is gone (the agent stopped meanwhile: the caller
    /// gives the task back).
    pub fn fill(&self, held: Held) -> bool {
        let mut st = self.st();
        match st.held.get_mut(&held.given.agent) {
            Some(slot @ None) => {
                *slot = Some(held);
                true
            }
            _ => false,
        }
    }

    /// Lets go of a slot [`Self::begin`] reserved and never filled (the claim was refused).
    pub fn cancel(&self, agent: &AgentId) {
        let mut st = self.st();
        if matches!(st.held.get(agent), Some(None)) {
            st.held.remove(agent);
        }
    }

    /// Remembers a dispatch prompt queued behind a structured agent's turn under the queue's `tag`.
    pub fn queued(&self, tag: u64, given: Given) {
        self.st().queued.insert((given.agent.clone(), tag), given);
    }

    /// A queued prompt left the queue: sent (forgotten) or not (its task, to give back).
    pub fn dequeued(&self, agent: &AgentId, tag: u64, sent: bool) -> Option<Given> {
        let g = self.st().queued.remove(&(agent.clone(), tag));
        g.filter(|_| !sent)
    }

    /// Queued prompts that will never be sent: their tasks, to give back.
    pub fn dropped(&self, agent: &AgentId, tags: &[u64]) -> Vec<Given> {
        let mut st = self.st();
        tags.iter()
            .filter_map(|t| st.queued.remove(&(agent.clone(), *t)))
            .collect()
    }

    /// One round of the supervision loop. Every dispatch of an agent that is no longer running (stopped by a person, by its parent,
    /// ended, crashed, gone) is dropped: its prompt is never pasted. Then the held prompt of each running terminal agent that is between
    /// turns and settled is handed to `deliver` (which pastes it, off this thread, and says whether it could start). Returns the tasks to
    /// give back: the dropped ones and those `deliver` could not start.
    pub fn tick(
        &self,
        agents: &[Seen],
        now: Instant,
        mut deliver: impl FnMut(&Held) -> bool,
    ) -> Vec<Given> {
        let running: HashSet<&AgentId> =
            agents.iter().filter(|a| a.running).map(|a| &a.id).collect();
        let mut back = Vec::new();
        let due = {
            let mut st = self.st();
            st.pasted
                .retain(|a, t| running.contains(a) && now.saturating_duration_since(*t) < SETTLE);
            let gone: Vec<AgentId> = st
                .held
                .keys()
                .filter(|a| !running.contains(a))
                .cloned()
                .collect();
            for a in gone {
                if let Some(Some(h)) = st.held.remove(&a) {
                    back.push(h.given);
                }
            }
            let gone: Vec<(AgentId, u64)> = st
                .queued
                .keys()
                .filter(|(a, _)| !running.contains(a))
                .cloned()
                .collect();
            for k in gone {
                back.extend(st.queued.remove(&k));
            }
            let mut due = Vec::new();
            for a in agents.iter().filter(|a| a.running && a.terminal) {
                let ready = between_turns(a.state)
                    && !st.pasted.contains_key(&a.id)
                    && matches!(st.held.get(&a.id), Some(Some(_)));
                if ready {
                    if let Some(Some(h)) = st.held.remove(&a.id) {
                        st.pasted.insert(a.id.clone(), now);
                        due.push(h);
                    }
                }
            }
            due
        };
        for h in due {
            if !deliver(&h) {
                back.push(h.given);
            }
        }
        back
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn given(agent: &str, task: &str) -> Given {
        Given {
            agent: AgentId::new(agent),
            task_id: task.into(),
            shared: false,
        }
    }

    fn held(agent: &str, task: &str) -> Held {
        Held {
            prompt: format!("prompt {task}"),
            given: given(agent, task),
        }
    }

    fn seen(id: &str, state: AgentState, running: bool) -> Seen {
        Seen {
            id: AgentId::new(id),
            state,
            running,
            terminal: true,
        }
    }

    #[test]
    fn two_dispatches_at_once_never_both_paste_and_a_busy_agent_gets_one_held() {
        let d = Dispatches::default();
        let a = AgentId::new("a");
        let t0 = Instant::now();
        // Idle: the first pastes; the second, at the same moment, is held (the first paste is counted at once).
        assert_eq!(d.begin(&a, AgentState::Idle, t0).unwrap(), Slot::PasteNow);
        assert_eq!(d.begin(&a, AgentState::Idle, t0).unwrap(), Slot::Hold);
        // The slot is reserved before any claim: a third is refused, whatever happens to the second's claim.
        let e = d.begin(&a, AgentState::Idle, t0).unwrap_err();
        assert_eq!(e.code, ErrorCode::AgentQueueFull);
        assert_eq!(
            e.params.get("limit"),
            Some(&agora_protocol::ParamValue::Int(1))
        );
        // A refused claim lets the slot go.
        d.cancel(&a);
        assert_eq!(d.begin(&a, AgentState::Working, t0).unwrap(), Slot::Hold);
        assert!(d.fill(held("a", "t1")));
        // Filled: cancel does not empty it.
        d.cancel(&a);
        assert!(d.begin(&a, AgentState::Idle, t0).is_err());
    }

    #[test]
    fn the_supervision_round_pastes_a_held_prompt_only_between_turns_and_settled() {
        let d = Dispatches::default();
        let a = AgentId::new("a");
        let t0 = Instant::now();
        assert_eq!(d.begin(&a, AgentState::Blocked, t0).unwrap(), Slot::Hold);
        assert!(d.fill(held("a", "t1")));
        let mut pasted = Vec::new();
        // In a turn or waiting for an answer: nothing.
        for state in [AgentState::Working, AgentState::Blocked] {
            let back = d.tick(&[seen("a", state, true)], t0, |h| {
                pasted.push(h.clone());
                true
            });
            assert!(back.is_empty() && pasted.is_empty());
        }
        // Idle: pasted once.
        let back = d.tick(&[seen("a", AgentState::Idle, true)], t0, |h| {
            pasted.push(h.clone());
            true
        });
        assert!(back.is_empty());
        assert_eq!(pasted, vec![held("a", "t1")]);
        // Right after, a new dispatch is held, and not pasted until SETTLE has passed, even though the screen still looks idle.
        assert_eq!(
            d.begin(&a, AgentState::Idle, t0 + Duration::from_secs(1))
                .unwrap(),
            Slot::Hold
        );
        assert!(d.fill(held("a", "t2")));
        let mut later = Vec::new();
        d.tick(
            &[seen("a", AgentState::Idle, true)],
            t0 + Duration::from_secs(1),
            |h| {
                later.push(h.clone());
                true
            },
        );
        assert!(later.is_empty());
        d.tick(&[seen("a", AgentState::Idle, true)], t0 + SETTLE, |h| {
            later.push(h.clone());
            true
        });
        assert_eq!(later, vec![held("a", "t2")]);
        // A paste that cannot start gives its task back.
        assert_eq!(
            d.begin(&a, AgentState::Working, t0 + SETTLE).unwrap(),
            Slot::Hold
        );
        assert!(d.fill(held("a", "t3")));
        let back = d.tick(
            &[seen("a", AgentState::Idle, true)],
            t0 + SETTLE * 2,
            |_| false,
        );
        assert_eq!(back, vec![given("a", "t3")]);
    }

    #[test]
    fn a_stopped_agents_dispatches_are_dropped_and_their_tasks_come_back() {
        let d = Dispatches::default();
        let a = AgentId::new("a");
        let t0 = Instant::now();
        assert_eq!(d.begin(&a, AgentState::Working, t0).unwrap(), Slot::Hold);
        assert!(d.fill(held("a", "t1")));
        d.queued(7, given("b", "t2"));
        // A reserved slot of an agent that stops while its dispatch claims: dropped, so the late fill fails (the caller gives back).
        let c = AgentId::new("c");
        assert_eq!(d.begin(&c, AgentState::Working, t0).unwrap(), Slot::Hold);
        // Stopped (no longer running) or gone (not listed): every dispatch of theirs is dropped, never pasted.
        let mut back = d.tick(&[seen("a", AgentState::Idle, false)], t0, |_| {
            panic!("a stopped agent's prompt was pasted")
        });
        back.sort_by(|x, y| x.task_id.cmp(&y.task_id));
        assert_eq!(back, vec![given("a", "t1"), given("b", "t2")]);
        assert!(!d.fill(held("c", "t3")));
        // The slot is free again for a new start of the agent.
        assert_eq!(d.begin(&a, AgentState::Idle, t0).unwrap(), Slot::PasteNow);
    }

    #[test]
    fn a_queued_prompt_that_is_not_sent_gives_its_task_back() {
        let d = Dispatches::default();
        let a = AgentId::new("a");
        d.queued(1, given("a", "t1"));
        d.queued(2, given("a", "t2"));
        d.queued(3, given("a", "t3"));
        assert_eq!(d.dequeued(&a, 1, true), None);
        assert_eq!(d.dequeued(&a, 2, false), Some(given("a", "t2")));
        assert_eq!(d.dropped(&a, &[3, 99]), vec![given("a", "t3")]);
        assert_eq!(d.dequeued(&a, 1, false), None);
    }

    #[test]
    fn a_paste_cannot_be_ended_by_the_task_text() {
        let prompt = agora_mesh::dispatch::build_prompt(
            "p",
            "t1",
            "x\u{1b}[201~ title",
            "a\u{1b}[201~\nrm -rf /\r\n\u{9b}201~ end",
            16384,
        );
        assert!(!prompt.contains('\u{1b}') && !prompt.contains('\u{9b}'));
        let bytes = paste_bytes(&prompt);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("\u{1b}[200~"));
        assert_eq!(text.matches("\u{1b}[201~").count(), 1);
        assert!(text.ends_with("\u{1b}[201~"));
        assert_eq!(paste_bytes("a\u{1b}b"), b"\x1b[200~ab\x1b[201~".to_vec());
    }

    #[test]
    fn the_prompt_hash_is_lowercase_hex_sha256() {
        assert_eq!(
            prompt_sha256("test"),
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        );
    }
}
