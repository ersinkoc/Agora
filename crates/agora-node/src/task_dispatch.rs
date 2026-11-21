//! ADR-0095 decision 5 on the node: a task's prompt is not typed over a running turn, as far as the node can tell (see
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

/// How long a terminal agent's dispatch prompt is held at most. An agent that stays in a turn, waits for an answer or shows a screen
/// no rule names for longer gets it dropped, and a task on the node's broker back, so a stuck agent does not keep a task and its slot
/// for good.
pub const HOLD_FOR: Duration = Duration::from_secs(15 * 60);

/// Identifies one reservation ([`Dispatches::reserve`]): a late call of an older dispatch to an agent of the same id never acts on a
/// newer dispatch's reservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token(u64);

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

/// What [`Dispatches::decide`] decided for a terminal agent, after its task was claimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// Between turns and settled: paste now. The reservation is let go and the paste counted at once, so another dispatch at the
    /// same moment is held.
    PasteNow,
    /// Not between turns: the reservation stays; fill it with [`Dispatches::fill`].
    Hold,
}

/// One agent as the supervision loop sees it, for [`Dispatches::tick`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub id: AgentId,
    pub state: AgentState,
    pub running: bool,
    pub terminal: bool,
    /// Its kind has a screen-detection rule that says idle or done, so it can leave `unknown` for a state the node pastes in.
    pub detects: bool,
}

/// Whether a terminal agent in `state` is between turns, so a prompt may be pasted: idle, or finished and not yet seen. `Unknown`
/// counts only for a kind with no rule that says idle or done (`detects` false), which never reaches either: without that its held
/// prompts would never arrive. A kind with rules that is `unknown` (starting up, or a screen no rule names, such as a dialog) is
/// waited for: the paste ends with Enter, which could answer a dialog nobody recognised.
pub fn between_turns(state: AgentState, detects: bool) -> bool {
    match state {
        AgentState::Idle | AgentState::Done => true,
        AgentState::Unknown => !detects,
        AgentState::Working | AgentState::Blocked => false,
    }
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

/// ADR-0095: pastes a dispatch prompt ([`paste_bytes`]), then, after a short pause, Enter. Blocks for the pause. A write fails only
/// when the terminal is gone (its program ended), so a failure after the paste leaves no input box holding the text; the caller gives
/// the task back either way.
pub fn paste_prompt(control: &agora_platform::PtyControl, prompt: &str) -> io::Result<()> {
    control.write_input(&paste_bytes(prompt))?;
    std::thread::sleep(Duration::from_millis(150));
    control.write_input(b"\r")
}

/// A terminal agent's hold slot.
struct Slotted {
    /// `None` while a dispatch reserved it and is claiming its task.
    held: Option<Held>,
    /// When it was reserved: a supervision round drops it only with a view of the agents taken after this, and it expires
    /// [`HOLD_FOR`] after.
    since: Instant,
    token: Token,
}

#[derive(Default)]
struct State {
    held: HashMap<AgentId, Slotted>,
    /// The last reservation token given.
    tokens: u64,
    /// When a prompt was last pasted into each terminal agent (kept for [`SETTLE`]).
    pasted: HashMap<AgentId, Instant>,
    /// Dispatch prompts queued behind a structured agent's turn, by the tag the queue gave them. Forgotten only by the queue's own
    /// events ([`Dispatches::dequeued`], [`Dispatches::dropped`]): every way a queue ends (a stop, an exit, a crash, a cancel that drops
    /// it) reports its prompts, and a prompt reported sent must keep its claim even if the agent ended in the same round.
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

    /// Whether nothing is reserved, held or queued (the supervision loop then has nothing to do for dispatches).
    pub fn is_empty(&self) -> bool {
        let st = self.st();
        st.held.is_empty() && st.queued.is_empty()
    }

    /// Reserves terminal agent `agent`'s one hold slot before its task is claimed, so a second dispatch meanwhile is refused with
    /// `agent.queue_full` (`limit` 1) and claims nothing. After the claim, [`Self::decide`]; on a refused claim, [`Self::cancel`].
    pub fn reserve(&self, agent: &AgentId, now: Instant) -> Result<Token, Error> {
        let mut st = self.st();
        if st.held.contains_key(agent) {
            return Err(queue_full());
        }
        st.tokens += 1;
        let token = Token(st.tokens);
        st.held.insert(
            agent.clone(),
            Slotted {
                held: None,
                since: now,
                token,
            },
        );
        Ok(token)
    }

    /// After the claim, with the agent's state read again (`state`, `detects`): paste now when it is between turns and nothing was
    /// pasted into it in the last [`SETTLE`] (the reservation goes and the paste is counted at once), else hold. `None` when the
    /// reservation is gone (the agent stopped meanwhile: the caller gives the task back).
    pub fn decide(
        &self,
        agent: &AgentId,
        token: Token,
        state: AgentState,
        detects: bool,
        now: Instant,
    ) -> Option<Slot> {
        let mut st = self.st();
        if !matches!(st.held.get(agent), Some(Slotted { held: None, token: t, .. }) if *t == token)
        {
            return None;
        }
        let settled = st
            .pasted
            .get(agent)
            .is_none_or(|t| now.saturating_duration_since(*t) >= SETTLE);
        if between_turns(state, detects) && settled {
            st.held.remove(agent);
            st.pasted.insert(agent.clone(), now);
            Some(Slot::PasteNow)
        } else {
            Some(Slot::Hold)
        }
    }

    /// Fills the slot reserved for `held.given.agent`; false when it is gone (the agent stopped meanwhile: the caller gives the task
    /// back).
    pub fn fill(&self, token: Token, held: Held) -> bool {
        let mut st = self.st();
        match st.held.get_mut(&held.given.agent) {
            Some(slot @ Slotted { held: None, .. }) if slot.token == token => {
                slot.held = Some(held);
                true
            }
            _ => false,
        }
    }

    /// Lets go of a reservation never filled (the claim was refused).
    pub fn cancel(&self, agent: &AgentId, token: Token) {
        let mut st = self.st();
        if matches!(st.held.get(agent), Some(Slotted { held: None, token: t, .. }) if *t == token) {
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

    /// One round of the supervision loop, with `agents` as the supervisor listed them at `seen_at`. A terminal agent's held or reserved
    /// dispatch made before `seen_at` whose agent that list shows not running (stopped by a person or its parent, ended, crashed, gone)
    /// is dropped and never pasted; one made after it is left for the next round (the list may predate a just-started agent). One held
    /// longer than [`HOLD_FOR`] is dropped too. Queued prompts of structured agents are left to the queue's events. Then the held prompt
    /// of each running terminal agent that is between turns and settled is handed to `deliver` (which pastes it, off this thread, and
    /// says whether it could start). Returns the tasks to give back: the dropped ones and those `deliver` could not start.
    pub fn tick(
        &self,
        agents: &[Seen],
        seen_at: Instant,
        now: Instant,
        mut deliver: impl FnMut(&Held) -> bool,
    ) -> Vec<Given> {
        let running: HashSet<&AgentId> =
            agents.iter().filter(|a| a.running).map(|a| &a.id).collect();
        let mut back = Vec::new();
        let due = {
            let mut st = self.st();
            st.pasted
                .retain(|_, t| now.saturating_duration_since(*t) < SETTLE);
            let gone: Vec<AgentId> = st
                .held
                .iter()
                .filter(|(a, s)| {
                    (!running.contains(a) && s.since < seen_at)
                        || now.saturating_duration_since(s.since) >= HOLD_FOR
                })
                .map(|(a, _)| a.clone())
                .collect();
            for a in gone {
                if let Some(Slotted { held: Some(h), .. }) = st.held.remove(&a) {
                    back.push(h.given);
                }
            }
            let mut due = Vec::new();
            for a in agents.iter().filter(|a| a.running && a.terminal) {
                let ready = between_turns(a.state, a.detects)
                    && !st.pasted.contains_key(&a.id)
                    && matches!(st.held.get(&a.id), Some(Slotted { held: Some(_), .. }));
                if ready {
                    if let Some(Slotted { held: Some(h), .. }) = st.held.remove(&a.id) {
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
            detects: true,
        }
    }

    /// Reserves, decides `Hold` and fills: a prompt held for `agent`.
    fn hold(d: &Dispatches, agent: &str, task: &str, now: Instant) {
        let a = AgentId::new(agent);
        let t = d.reserve(&a, now).unwrap();
        assert_eq!(
            d.decide(&a, t, AgentState::Working, true, now),
            Some(Slot::Hold)
        );
        assert!(d.fill(t, held(agent, task)));
    }

    #[test]
    fn unknown_is_between_turns_only_for_a_kind_that_cannot_say_idle() {
        assert!(between_turns(AgentState::Idle, true));
        assert!(between_turns(AgentState::Done, true));
        assert!(!between_turns(AgentState::Unknown, true));
        assert!(between_turns(AgentState::Unknown, false));
        assert!(!between_turns(AgentState::Working, false));
        assert!(!between_turns(AgentState::Blocked, false));
    }

    #[test]
    fn the_slot_is_reserved_before_the_claim_and_decided_again_after_it() {
        let d = Dispatches::default();
        let a = AgentId::new("a");
        let t0 = Instant::now();
        assert!(d.is_empty());
        let t = d.reserve(&a, t0).unwrap();
        // While the first claims, a second is refused and claims nothing.
        let e = d.reserve(&a, t0).unwrap_err();
        assert_eq!(e.code, ErrorCode::AgentQueueFull);
        assert_eq!(
            e.params.get("limit"),
            Some(&agora_protocol::ParamValue::Int(1))
        );
        // Decided after the claim with the state read again: a turn began meanwhile, so it is held, not pasted over it.
        assert_eq!(
            d.decide(&a, t, AgentState::Working, true, t0),
            Some(Slot::Hold)
        );
        assert!(d.fill(t, held("a", "t1")));
        assert!(d.reserve(&a, t0).is_err());
        // A refused claim lets a reservation go; a filled slot stays.
        d.cancel(&a, t);
        assert!(!d.is_empty());
        let b = AgentId::new("b");
        let tb = d.reserve(&b, t0).unwrap();
        d.cancel(&b, tb);
        let tb = d.reserve(&b, t0).unwrap();
        // Between turns at decision time: pasted, the reservation goes and the paste counts at once.
        assert_eq!(
            d.decide(&b, tb, AgentState::Idle, true, t0),
            Some(Slot::PasteNow)
        );
        assert_eq!(d.decide(&b, tb, AgentState::Idle, true, t0), None);
        let tb = d.reserve(&b, t0).unwrap();
        // Within SETTLE of that paste a second is held even though the screen still looks idle.
        assert_eq!(
            d.decide(&b, tb, AgentState::Idle, true, t0 + Duration::from_secs(1)),
            Some(Slot::Hold)
        );
    }

    #[test]
    fn an_older_dispatch_never_touches_a_newer_reservation_of_the_same_agent() {
        let d = Dispatches::default();
        let a = AgentId::new("a");
        let t0 = Instant::now();
        let old = d.reserve(&a, t0).unwrap();
        // The agent stops (the round drops the reservation) and is resumed under the same id; a new dispatch reserves.
        let later = t0 + Duration::from_millis(5);
        d.tick(&[seen("a", AgentState::Idle, false)], later, later, |_| {
            true
        });
        let new = d.reserve(&a, later).unwrap();
        // The old dispatch's late calls do nothing to it.
        d.cancel(&a, old);
        assert_eq!(d.decide(&a, old, AgentState::Idle, true, later), None);
        assert!(!d.fill(old, held("a", "old")));
        assert_eq!(
            d.decide(&a, new, AgentState::Working, true, later),
            Some(Slot::Hold)
        );
        assert!(d.fill(new, held("a", "new")));
    }

    #[test]
    fn the_supervision_round_pastes_a_held_prompt_only_between_turns_and_settled() {
        let d = Dispatches::default();
        let a = AgentId::new("a");
        let t0 = Instant::now();
        hold(&d, "a", "t1", t0);
        let later = t0 + Duration::from_millis(10);
        let mut pasted = Vec::new();
        // In a turn, waiting for an answer, or unknown for a kind that can say idle: nothing.
        for state in [
            AgentState::Working,
            AgentState::Blocked,
            AgentState::Unknown,
        ] {
            let back = d.tick(&[seen("a", state, true)], later, later, |h| {
                pasted.push(h.clone());
                true
            });
            assert!(back.is_empty() && pasted.is_empty(), "{state:?}");
        }
        // Idle: pasted once.
        let back = d.tick(&[seen("a", AgentState::Idle, true)], later, later, |h| {
            pasted.push(h.clone());
            true
        });
        assert!(back.is_empty());
        assert_eq!(pasted, vec![held("a", "t1")]);
        // A new dispatch right after is held, and not pasted until SETTLE has passed.
        let t = d.reserve(&a, later).unwrap();
        assert_eq!(
            d.decide(&a, t, AgentState::Idle, true, later),
            Some(Slot::Hold)
        );
        assert!(d.fill(t, held("a", "t2")));
        let mut next = Vec::new();
        let t1 = later + Duration::from_secs(1);
        d.tick(&[seen("a", AgentState::Idle, true)], t1, t1, |h| {
            next.push(h.clone());
            true
        });
        assert!(next.is_empty());
        let t2 = later + SETTLE;
        d.tick(&[seen("a", AgentState::Idle, true)], t2, t2, |h| {
            next.push(h.clone());
            true
        });
        assert_eq!(next, vec![held("a", "t2")]);
        // A kind that cannot say idle is pasted to while unknown; a paste that cannot start gives its task back.
        let mut free = seen("a", AgentState::Unknown, true);
        free.detects = false;
        hold(&d, "a", "t3", t2);
        let t3 = t2 + SETTLE;
        let back = d.tick(&[free], t3, t3, |_| false);
        assert_eq!(back, vec![given("a", "t3")]);
    }

    #[test]
    fn a_held_prompt_expires_and_its_task_comes_back() {
        let d = Dispatches::default();
        let t0 = Instant::now();
        hold(&d, "a", "t1", t0);
        let stuck = [seen("a", AgentState::Blocked, true)];
        let almost = t0 + HOLD_FOR - Duration::from_secs(1);
        assert!(d.tick(&stuck, almost, almost, |_| true).is_empty());
        let over = t0 + HOLD_FOR;
        assert_eq!(d.tick(&stuck, over, over, |_| true), vec![given("a", "t1")]);
        assert!(d.is_empty());
    }

    #[test]
    fn a_stopped_agents_held_dispatches_are_dropped_and_their_tasks_come_back() {
        let d = Dispatches::default();
        let t0 = Instant::now();
        hold(&d, "a", "t1", t0);
        // A reservation of an agent that stops while its dispatch claims: dropped, so the late decision finds nothing (the caller
        // gives back).
        let c = AgentId::new("c");
        let tc = d.reserve(&c, t0).unwrap();
        let seen_at = t0 + Duration::from_millis(5);
        let back = d.tick(
            &[seen("a", AgentState::Idle, false)],
            seen_at,
            seen_at,
            |_| panic!("a stopped agent's prompt was pasted"),
        );
        assert_eq!(back, vec![given("a", "t1")]);
        assert_eq!(d.decide(&c, tc, AgentState::Idle, true, seen_at), None);
        assert!(!d.fill(tc, held("c", "t3")));
        assert!(d.is_empty());
    }

    #[test]
    fn a_queued_prompt_sent_just_before_its_agent_ended_keeps_its_claim() {
        // In one round an adapter reports the queued dispatch prompt sent and then exits: the round's list shows the agent gone, but
        // only the queue's events decide; sent means the agent has the task.
        let d = Dispatches::default();
        let a = AgentId::new("a");
        let t0 = Instant::now();
        d.queued(1, given("a", "t1"));
        d.queued(2, given("a", "t2"));
        let later = t0 + Duration::from_millis(5);
        let back = d.tick(&[seen("a", AgentState::Idle, false)], later, later, |_| {
            true
        });
        assert!(
            back.is_empty(),
            "the round gave back a queued task: {back:?}"
        );
        assert_eq!(d.dequeued(&a, 1, true), None);
        // The exit reports what was still queued.
        assert_eq!(d.dropped(&a, &[2]), vec![given("a", "t2")]);
        assert!(d.is_empty());
    }

    #[test]
    fn a_list_older_than_a_dispatch_does_not_drop_it() {
        // The supervision round's list was taken before a just-started agent existed; the dispatch to it came after.
        let d = Dispatches::default();
        let a = AgentId::new("new");
        let seen_at = Instant::now();
        let t0 = seen_at + Duration::from_millis(5);
        let t = d.reserve(&a, t0).unwrap();
        assert!(d.tick(&[], seen_at, t0, |_| true).is_empty());
        assert_eq!(
            d.decide(&a, t, AgentState::Idle, true, t0),
            Some(Slot::PasteNow)
        );
        // Its paste mark survives a round with an old list, so a second dispatch is still held.
        d.tick(&[], seen_at, t0, |_| true);
        let t = d.reserve(&a, t0).unwrap();
        assert_eq!(
            d.decide(&a, t, AgentState::Idle, true, t0),
            Some(Slot::Hold)
        );
        assert!(d.fill(t, held("new", "t9")));
        // A newer list that shows it gone drops it.
        let after = t0 + Duration::from_millis(5);
        assert_eq!(
            d.tick(&[], after, after, |_| true),
            vec![given("new", "t9")]
        );
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
        assert!(d.is_empty());
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
        let text = String::from_utf8(paste_bytes(&prompt)).unwrap();
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
